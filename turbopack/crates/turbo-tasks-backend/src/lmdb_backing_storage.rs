mod extended_key;

use std::{
    cell::UnsafeCell,
    collections::{hash_map::Entry, HashMap},
    env,
    error::Error,
    fs::{self, create_dir_all, metadata, read_dir, remove_dir_all},
    mem::{transmute, ManuallyDrop},
    path::Path,
    sync::Arc,
    thread::available_parallelism,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use lmdb::{
    Database, DatabaseFlags, Environment, EnvironmentFlags, RoTransaction, Transaction, WriteFlags,
};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use smallvec::SmallVec;
use thread_local::ThreadLocal;
use tracing::Span;
use turbo_tasks::{backend::CachedTaskType, KeyValuePair, SessionId, TaskId};

use crate::{
    backend::{AnyOperation, TaskDataCategory},
    backing_storage::{BackingStorage, ReadTransaction},
    data::{CachedDataItem, CachedDataItemKey, CachedDataItemValue, CachedDataUpdate},
    utils::chunked_vec::ChunkedVec,
};

const META_KEY_OPERATIONS: u32 = 0;
const META_KEY_NEXT_FREE_TASK_ID: u32 = 1;
const META_KEY_SESSION_ID: u32 = 2;

/// Specifies many databases that have a different version than the current one are retained.
/// For example if MAX_OTHER_DB_VERSIONS is 2, there can be at most 3 databases in the directory,
/// the current one and two older/newer ones.
const MAX_OTHER_DB_VERSIONS: usize = 2;

struct IntKey([u8; 4]);

impl IntKey {
    fn new(value: u32) -> Self {
        Self(value.to_be_bytes())
    }
}

impl AsRef<[u8]> for IntKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

fn as_u32<E: Error + Send + Sync + 'static>(result: Result<&[u8], E>) -> Result<u32> {
    let bytes = result?;
    let n = bytes.try_into().map(u32::from_be_bytes)?;
    Ok(n)
}

struct ThreadLocalReadTransactionsContainer(UnsafeCell<SmallVec<[RoTransaction<'static>; 4]>>);

impl ThreadLocalReadTransactionsContainer {
    unsafe fn pop(&self) -> Option<RoTransaction<'static>> {
        // Safety: Access only happens via `push` and `pop`, and
        // `ThreadLocalReadTransactionsContainer` is not `Sync`, so we can know we can know this
        // block is the only one currently reading or writing to the cell.
        let vec = unsafe { &mut *self.0.get() };
        vec.pop()
    }

    unsafe fn push(&self, tx: RoTransaction<'static>) {
        // Safety: Access only happens via `push` and `pop`, and
        // `ThreadLocalReadTransactionsContainer` is not `Sync`, so we can know we can know this
        // block is the only one currently reading or writing to the cell.
        let vec = unsafe { &mut *self.0.get() };
        vec.push(tx)
    }
}

// Safety: It's safe to send `RoTransaction` between threads as we construct `Environment` with
// `EnvironmentFlags::NO_TLS`, but the types don't allow that.
unsafe impl Send for ThreadLocalReadTransactionsContainer {}

pub struct LmdbBackingStorage {
    // Safety: `read_transactions_cache` need to be dropped before `env` since it will end the
    // transactions.
    read_transactions_cache: ArcSwap<ThreadLocal<ThreadLocalReadTransactionsContainer>>,
    env: Environment,
    infra_db: Database,
    data_db: Database,
    meta_db: Database,
    forward_task_cache_db: Database,
    reverse_task_cache_db: Database,
    fresh_db: bool,
}

impl LmdbBackingStorage {
    pub fn new(base_path: &Path) -> Result<Self> {
        // Database versioning. Pass `TURBO_ENGINE_IGNORE_DIRTY` at runtime to ignore a
        // dirty git repository. Pass `TURBO_ENGINE_DISABLE_VERSIONING` at runtime to disable
        // versioning and always use the same database.
        let version_info = env!("VERGEN_GIT_DESCRIBE");
        let (version_info, git_dirty) =
            if let Some(version_info) = version_info.strip_suffix("-dirty") {
                (version_info, true)
            } else {
                (version_info, false)
            };
        let ignore_dirty = env::var("TURBO_ENGINE_IGNORE_DIRTY").ok().is_some();
        let disabled_versioning = env::var("TURBO_ENGINE_DISABLE_VERSIONING").ok().is_some();
        let version = if disabled_versioning {
            println!(
                "WARNING: Persistent Caching versioning is disabled. Manual removal of the \
                 persistent caching database might be required."
            );
            Some("unversioned")
        } else if !git_dirty {
            Some(version_info)
        } else if ignore_dirty {
            println!(
                "WARNING: The git repository is dirty, but Persistent Caching is still enabled. \
                 Manual removal of the persistent caching database might be required."
            );
            Some(version_info)
        } else {
            println!(
                "WARNING: The git repository is dirty: Persistent Caching is disabled. Use \
                 TURBO_ENGINE_IGNORE_DIRTY=1 to ignore dirtyness of the repository."
            );
            None
        };
        let path;
        if let Some(version) = version {
            path = base_path.join(version);

            // Remove old databases if needed
            if let Ok(read_dir) = read_dir(base_path) {
                let old_dbs = read_dir
                    .filter_map(|entry| {
                        let entry = entry.ok()?;
                        if !entry.file_type().ok()?.is_dir() {
                            return None;
                        }
                        let name = entry.file_name();
                        let name = name.to_string_lossy();
                        if name == version {
                            return None;
                        }
                        Some(entry.path())
                    })
                    .collect::<Vec<_>>();
                if old_dbs.len() > MAX_OTHER_DB_VERSIONS {
                    let mut old_dbs = old_dbs
                        .iter()
                        .map(|p| {
                            fn get_age(p: &Path) -> Result<Duration> {
                                let m = metadata(p)?;
                                Ok(m.accessed().or_else(|_| m.modified())?.elapsed()?)
                            }
                            (
                                p,
                                get_age(p).unwrap_or(Duration::from_secs(10 * 356 * 24 * 60 * 60)),
                            )
                        })
                        .collect::<Vec<_>>();
                    old_dbs.sort_by_key(|(_, age)| *age);
                    for (p, _) in old_dbs.into_iter().skip(MAX_OTHER_DB_VERSIONS) {
                        let _ = remove_dir_all(p);
                    }
                }
            }
        } else {
            let _ = remove_dir_all(base_path);
            path = base_path.join("temp");
        }
        let fresh_db = fs::exists(&path).map_or(false, |exists| !exists);
        if fresh_db {
            create_dir_all(&path).context("Creating database directory failed")?;
        }

        #[cfg(target_arch = "x86")]
        const MAP_SIZE: usize = usize::MAX;
        #[cfg(not(target_arch = "x86"))]
        const MAP_SIZE: usize = 40 * 1024 * 1024 * 1024;

        let env = Environment::new()
            .set_flags(
                EnvironmentFlags::WRITE_MAP
                    | EnvironmentFlags::NO_META_SYNC
                    | EnvironmentFlags::NO_TLS,
            )
            .set_max_readers((available_parallelism().map_or(16, |v| v.get()) * 8) as u32)
            .set_max_dbs(5)
            .set_map_size(MAP_SIZE)
            .open(&path)?;
        let infra_db = env.create_db(Some("infra"), DatabaseFlags::INTEGER_KEY)?;
        let data_db = env.create_db(Some("data"), DatabaseFlags::INTEGER_KEY)?;
        let meta_db = env.create_db(Some("meta"), DatabaseFlags::INTEGER_KEY)?;
        let forward_task_cache_db =
            env.create_db(Some("forward_task_cache"), DatabaseFlags::empty())?;
        let reverse_task_cache_db =
            env.create_db(Some("reverse_task_cache"), DatabaseFlags::INTEGER_KEY)?;
        Ok(Self {
            env,
            infra_db,
            data_db,
            meta_db,
            forward_task_cache_db,
            reverse_task_cache_db,
            fresh_db,
            read_transactions_cache: ArcSwap::new(Arc::new(ThreadLocal::new())),
        })
    }

    fn db(&self, category: TaskDataCategory) -> Database {
        match category {
            TaskDataCategory::Meta => self.meta_db,
            TaskDataCategory::Data => self.data_db,
            _ => unreachable!(),
        }
    }

    fn begin_read_transaction(&self) -> Result<RoTransaction<'_>> {
        let guard = self.read_transactions_cache.load();
        let container = guard
            .get_or(|| ThreadLocalReadTransactionsContainer(UnsafeCell::new(Default::default())));
        // Safety: Since it's a thread local it's safe to take from the container
        Ok(if let Some(tx) = unsafe { container.pop() } {
            tx
        } else {
            self.env.begin_ro_txn()?
        })
    }

    fn end_read_transaction(&self, tx: RoTransaction<'_>) {
        let guard = self.read_transactions_cache.load();
        let container = guard
            .get_or(|| ThreadLocalReadTransactionsContainer(UnsafeCell::new(Default::default())));
        // Safety: We cast it to 'static lifetime, but it will be casted back to 'env when
        // taken. It's safe since this will not outlive the environment. We need to
        // be careful with Drop, but `read_transactions_cache` is before `env` in the
        // LmdbBackingStorage struct, so it's fine.
        let tx = unsafe { transmute::<RoTransaction<'_>, RoTransaction<'static>>(tx) };
        // Safety: It's safe to put it back since it's a thread local
        unsafe {
            container.push(tx);
        }
    }

    fn to_tx(&self, tx: ReadTransaction) -> ManuallyDrop<RoTransaction<'_>> {
        ManuallyDrop::new(unsafe { transmute::<*const (), RoTransaction<'_>>(tx.0) })
    }

    fn from_tx(tx: RoTransaction<'_>) -> ReadTransaction {
        ReadTransaction(unsafe { transmute::<RoTransaction<'_>, *const ()>(tx) })
    }

    fn with_tx<T>(
        &self,
        tx: Option<ReadTransaction>,
        f: impl FnOnce(&RoTransaction<'_>) -> Result<T>,
    ) -> Result<T> {
        if let Some(tx) = tx {
            let tx = self.to_tx(tx);
            f(&tx)
        } else {
            let tx = self.begin_read_transaction()?;
            let r = f(&tx)?;
            self.end_read_transaction(tx);
            Ok(r)
        }
    }
}

impl BackingStorage for LmdbBackingStorage {
    fn next_free_task_id(&self) -> TaskId {
        fn get(this: &LmdbBackingStorage) -> Result<u32> {
            let tx = this.env.begin_rw_txn()?;
            let next_free_task_id =
                as_u32(tx.get(this.infra_db, &IntKey::new(META_KEY_NEXT_FREE_TASK_ID)))?;
            Ok(next_free_task_id)
        }
        TaskId::from(get(self).unwrap_or(1))
    }

    fn next_session_id(&self) -> SessionId {
        fn get(this: &LmdbBackingStorage) -> Result<u32> {
            let tx = this.env.begin_rw_txn()?;
            let session_id = as_u32(tx.get(this.infra_db, &IntKey::new(META_KEY_SESSION_ID)))?;
            Ok(session_id)
        }
        SessionId::from(get(self).unwrap_or(0) + 1)
    }

    fn uncompleted_operations(&self) -> Vec<AnyOperation> {
        fn get(this: &LmdbBackingStorage) -> Result<Vec<AnyOperation>> {
            this.with_tx(None, |tx| {
                let operations = tx.get(this.infra_db, &IntKey::new(META_KEY_OPERATIONS))?;
                let operations = pot::from_slice(operations)?;
                Ok(operations)
            })
        }
        get(self).unwrap_or_default()
    }

    fn save_snapshot(
        &self,
        session_id: SessionId,
        operations: Vec<Arc<AnyOperation>>,
        task_cache_updates: Vec<ChunkedVec<(Arc<CachedTaskType>, TaskId)>>,
        meta_updates: Vec<ChunkedVec<CachedDataUpdate>>,
        data_updates: Vec<ChunkedVec<CachedDataUpdate>>,
    ) -> Result<()> {
        let span = tracing::trace_span!("save snapshot", session_id = ?session_id, operations = operations.len(), db_operation_count = tracing::field::Empty);
        let mut op_count = 0;
        let mut tx = self.env.begin_rw_txn()?;
        let mut task_meta_items_result = Ok(Vec::new());
        let mut task_data_items_result = Ok(Vec::new());

        turbo_tasks::scope(|s| {
            // Start organizing the updates in parallel
            s.spawn(|_| {
                let task_meta_updates = {
                    let _span = tracing::trace_span!(
                        "organize task meta",
                        updates = meta_updates.iter().map(|m| m.len()).sum::<usize>()
                    )
                    .entered();
                    organize_task_data(meta_updates)
                };
                let items_result = {
                    let _span = tracing::trace_span!(
                        "restore task meta",
                        tasks = task_meta_updates.iter().map(|m| m.len()).sum::<usize>()
                    )
                    .entered();
                    restore_task_data(self, self.meta_db, task_meta_updates)
                };
                task_meta_items_result = items_result.and_then(|items| {
                    let _span = tracing::trace_span!("serialize task meta").entered();
                    serialize_task_data(items)
                });
            });
            s.spawn(|_| {
                let task_data_updates = {
                    let _span = tracing::trace_span!(
                        "organize task data",
                        updates = data_updates.iter().map(|m| m.len()).sum::<usize>()
                    )
                    .entered();
                    organize_task_data(data_updates)
                };
                let items_result = {
                    let _span = tracing::trace_span!(
                        "restore task data",
                        tasks = task_data_updates.iter().map(|m| m.len()).sum::<usize>()
                    )
                    .entered();
                    restore_task_data(self, self.data_db, task_data_updates)
                };
                task_data_items_result = items_result.and_then(|items| {
                    let _span = tracing::trace_span!("serialize task data").entered();
                    serialize_task_data(items)
                });
            });

            {
                let _span =
                    tracing::trace_span!("update session id", session_id = ?session_id).entered();
                tx.put(
                    self.infra_db,
                    &IntKey::new(META_KEY_SESSION_ID),
                    &session_id.to_be_bytes(),
                    WriteFlags::empty(),
                )
                .with_context(|| anyhow!("Unable to write next session id"))?;
            }

            let mut next_task_id =
                as_u32(tx.get(self.infra_db, &IntKey::new(META_KEY_NEXT_FREE_TASK_ID)))
                    .unwrap_or(1);
            {
                let _span = tracing::trace_span!(
                    "update task cache",
                    items = task_cache_updates.iter().map(|m| m.len()).sum::<usize>()
                )
                .entered();
                for (task_type, task_id) in task_cache_updates.into_iter().flatten() {
                    let task_id = *task_id;
                    let task_type_bytes = pot::to_vec(&*task_type).with_context(|| {
                        anyhow!("Unable to serialize task cache key {task_type:?}")
                    })?;
                    #[cfg(feature = "verify_serialization")]
                    {
                        let deserialize: Result<CachedTaskType, _> =
                            serde_path_to_error::deserialize(
                                &mut pot::de::SymbolList::new()
                                    .deserializer_for_slice(&task_type_bytes)?,
                            );
                        if let Err(err) = deserialize {
                            println!(
                                "Task type would not be deserializable {task_id}: \
                                 {err:?}\n{task_type:#?}"
                            );
                            panic!("Task type would not be deserializable {task_id}: {err:?}");
                        }
                    }
                    extended_key::put(
                        &mut tx,
                        self.forward_task_cache_db,
                        &task_type_bytes,
                        &task_id.to_be_bytes(),
                        WriteFlags::empty(),
                    )
                    .with_context(|| {
                        anyhow!("Unable to write task cache {task_type:?} => {task_id}")
                    })?;
                    tx.put(
                        self.reverse_task_cache_db,
                        &IntKey::new(task_id),
                        &task_type_bytes,
                        WriteFlags::empty(),
                    )
                    .with_context(|| {
                        anyhow!("Unable to write task cache {task_id} => {task_type:?}")
                    })?;
                    op_count += 2;
                    next_task_id = next_task_id.max(task_id + 1);
                }
                tx.put(
                    self.infra_db,
                    &IntKey::new(META_KEY_NEXT_FREE_TASK_ID),
                    &next_task_id.to_be_bytes(),
                    WriteFlags::empty(),
                )
                .with_context(|| anyhow!("Unable to write next free task id"))?;
            }
            {
                let _span =
                    tracing::trace_span!("update operations", operations = operations.len())
                        .entered();
                let operations = pot::to_vec(&operations)
                    .with_context(|| anyhow!("Unable to serialize operations"))?;
                tx.put(
                    self.infra_db,
                    &IntKey::new(META_KEY_OPERATIONS),
                    &operations,
                    WriteFlags::empty(),
                )
                .with_context(|| anyhow!("Unable to write operations"))?;
                op_count += 2;
            }

            anyhow::Ok(())
        })?;

        for (db, task_items) in [
            (self.meta_db, task_meta_items_result?),
            (self.data_db, task_data_items_result?),
        ] {
            {
                let _span =
                    tracing::trace_span!("update task data", tasks = task_items.len()).entered();
                for (task_id, value) in task_items {
                    tx.put(db, &IntKey::new(*task_id), &value, WriteFlags::empty())
                        .with_context(|| anyhow!("Unable to write data items for {task_id}"))?;
                    op_count += 1;
                }
            }
        }
        {
            let _span = tracing::trace_span!("commit").entered();
            tx.commit()
                .with_context(|| anyhow!("Unable to commit operations"))?;
        }
        {
            let _span = tracing::trace_span!("swap read transactions").entered();
            // This resets the thread local storage for read transactions, read transactions are
            // eventually dropped, allowing DB to free up unused storage.
            self.read_transactions_cache
                .store(Arc::new(ThreadLocal::new()));
        }
        span.record("db_operation_count", op_count);
        Ok(())
    }

    fn start_read_transaction(&self) -> Option<ReadTransaction> {
        Some(Self::from_tx(self.begin_read_transaction().ok()?))
    }

    unsafe fn end_read_transaction(&self, transaction: ReadTransaction) {
        self.end_read_transaction(ManuallyDrop::into_inner(Self::to_tx(self, transaction)));
    }

    unsafe fn forward_lookup_task_cache(
        &self,
        tx: Option<ReadTransaction>,
        task_type: &CachedTaskType,
    ) -> Option<TaskId> {
        // Performance optimization when the database was empty
        // It's assumed that no cache entries are removed from the memory cache, but we might change
        // that in future.
        if self.fresh_db {
            return None;
        }
        fn lookup(
            this: &LmdbBackingStorage,
            tx: &RoTransaction<'_>,
            task_type: &CachedTaskType,
        ) -> Result<Option<TaskId>> {
            let task_type = pot::to_vec(task_type)?;
            let bytes = match extended_key::get(tx, this.forward_task_cache_db, &task_type) {
                Ok(result) => result,
                Err(err) => {
                    if err == lmdb::Error::NotFound {
                        return Ok(None);
                    } else {
                        return Err(err.into());
                    }
                }
            };
            let bytes = bytes.try_into()?;
            let id = TaskId::from(u32::from_be_bytes(bytes));
            Ok(Some(id))
        }
        let id = self
            .with_tx(tx, |tx| lookup(self, tx, task_type))
            .inspect_err(|err| println!("Looking up task id for {task_type:?} failed: {err:?}"))
            .ok()??;
        Some(id)
    }

    unsafe fn reverse_lookup_task_cache(
        &self,
        tx: Option<ReadTransaction>,
        task_id: TaskId,
    ) -> Option<Arc<CachedTaskType>> {
        // Performance optimization when the database was empty
        // It's assumed that no cache entries are removed from the memory cache, but we might change
        // that in future.
        if self.fresh_db {
            return None;
        }
        fn lookup(
            this: &LmdbBackingStorage,
            tx: &RoTransaction<'_>,
            task_id: TaskId,
        ) -> Result<Option<Arc<CachedTaskType>>> {
            let bytes = match tx.get(this.reverse_task_cache_db, &IntKey::new(*task_id)) {
                Ok(bytes) => bytes,
                Err(err) => {
                    if err == lmdb::Error::NotFound {
                        return Ok(None);
                    } else {
                        return Err(err.into());
                    }
                }
            };
            Ok(Some(pot::from_slice(bytes)?))
        }
        let result = self
            .with_tx(tx, |tx| lookup(self, tx, task_id))
            .inspect_err(|err| println!("Looking up task type for {task_id} failed: {err:?}"))
            .ok()??;
        Some(result)
    }

    unsafe fn lookup_data(
        &self,
        tx: Option<ReadTransaction>,
        task_id: TaskId,
        category: TaskDataCategory,
    ) -> Vec<CachedDataItem> {
        // Performance optimization when the database was empty
        // It's assumed that no cache entries are removed from the memory cache, but we might change
        // that in future.
        if self.fresh_db {
            return Vec::new();
        }
        fn lookup(
            this: &LmdbBackingStorage,
            tx: &RoTransaction<'_>,
            task_id: TaskId,
            category: TaskDataCategory,
        ) -> Result<Vec<CachedDataItem>> {
            let bytes = match tx.get(this.db(category), &IntKey::new(*task_id)) {
                Ok(bytes) => bytes,
                Err(err) => {
                    if err == lmdb::Error::NotFound {
                        return Ok(Vec::new());
                    } else {
                        return Err(err.into());
                    }
                }
            };
            let result: Vec<CachedDataItem> = pot::from_slice(bytes)?;
            Ok(result)
        }
        self.with_tx(tx, |tx| lookup(self, tx, task_id, category))
            .inspect_err(|err| println!("Looking up data for {task_id} failed: {err:?}"))
            .unwrap_or_default()
    }
}

type OrganizedTaskData = HashMap<
    TaskId,
    HashMap<CachedDataItemKey, (Option<CachedDataItemValue>, Option<CachedDataItemValue>)>,
>;
type ShardedOrganizedTaskData = Vec<OrganizedTaskData>;

fn organize_task_data(updates: Vec<ChunkedVec<CachedDataUpdate>>) -> ShardedOrganizedTaskData {
    let span = Span::current();
    updates
        .into_par_iter()
        .map(|updates| {
            let _span = span.clone().entered();
            let mut task_updates: OrganizedTaskData = HashMap::new();
            for CachedDataUpdate {
                task,
                key,
                value,
                old_value,
            } in updates.into_iter()
            {
                let data = task_updates.entry(task).or_default();
                match data.entry(key) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().1 = value;
                    }
                    Entry::Vacant(entry) => {
                        entry.insert((old_value, value));
                    }
                }
            }
            task_updates.retain(|_, data| {
                data.retain(|_, (old_value, value)| *old_value != *value);
                !data.is_empty()
            });
            task_updates
        })
        .collect()
}

fn restore_task_data(
    this: &LmdbBackingStorage,
    db: Database,
    task_updates: ShardedOrganizedTaskData,
) -> Result<Vec<(TaskId, Vec<CachedDataItem>)>> {
    let mut result = Vec::with_capacity(task_updates.iter().map(|m| m.len()).sum());

    let tx = this.env.begin_ro_txn()?;
    for (task, updates) in task_updates.into_iter().flatten() {
        let mut map;
        if let Ok(old_data) = tx.get(db, &IntKey::new(*task)) {
            let old_data: Vec<CachedDataItem> = match pot::from_slice(old_data) {
                Ok(d) => d,
                Err(_) => serde_path_to_error::deserialize(
                    &mut pot::de::SymbolList::new().deserializer_for_slice(old_data)?,
                )
                .with_context(|| {
                    anyhow!("Unable to deserialize old value of {task}: {old_data:?}")
                })?,
            };
            map = old_data
                .into_iter()
                .map(|item| item.into_key_and_value())
                .collect();
        } else {
            map = HashMap::new();
        }
        for (key, (_, value)) in updates {
            if let Some(value) = value {
                map.insert(key, value);
            } else {
                map.remove(&key);
            }
        }
        let vec = map
            .into_iter()
            .map(|(key, value)| CachedDataItem::from_key_and_value(key, value))
            .collect();
        result.push((task, vec));
    }

    Ok(result)
}

fn serialize_task_data(
    tasks: Vec<(TaskId, Vec<CachedDataItem>)>,
) -> Result<Vec<(TaskId, Vec<u8>)>> {
    tasks
        .into_iter()
        .map(|(task_id, mut data)| {
            let value = match pot::to_vec(&data) {
                #[cfg(not(feature = "verify_serialization"))]
                Ok(value) => value,
                _ => {
                    let mut error = Ok(());
                    data.retain(|item| {
                        let mut buf = Vec::<u8>::new();
                        let mut symbol_map = pot::ser::SymbolMap::new();
                        let mut serializer = symbol_map.serializer_for(&mut buf).unwrap();
                        if let Err(err) = serde_path_to_error::serialize(item, &mut serializer) {
                            if item.is_optional() {
                                #[cfg(feature = "verify_serialization")]
                                println!("Skipping non-serializable optional item: {item:?}");
                            } else {
                                error = Err(err).context({
                                    anyhow!(
                                        "Unable to serialize data item for {task_id}: {item:#?}"
                                    )
                                });
                            }
                            false
                        } else {
                            #[cfg(feature = "verify_serialization")]
                            {
                                let deserialize: Result<CachedDataItem, _> =
                                    serde_path_to_error::deserialize(
                                        &mut pot::de::SymbolList::new()
                                            .deserializer_for_slice(&buf)
                                            .unwrap(),
                                    );
                                if let Err(err) = deserialize {
                                    println!(
                                        "Data item would not be deserializable {task_id}: \
                                         {err:?}\n{item:#?}"
                                    );
                                    return false;
                                }
                            }
                            true
                        }
                    });
                    error?;

                    pot::to_vec(&data).with_context(|| {
                        anyhow!("Unable to serialize data items for {task_id}: {data:#?}")
                    })?
                }
            };
            Ok((task_id, value))
        })
        .collect()
}
