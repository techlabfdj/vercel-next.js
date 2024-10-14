use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};
use next_core::emit_assets;
use serde::{Deserialize, Serialize};
use turbo_tasks::{
    debug::ValueDebugFormat, trace::TraceRawVcs, Completion, FxIndexSet, RcStr, State,
    TryFlatJoinIterExt, TryJoinIterExt, ValueDefault, ValueToString, Vc,
};
use turbo_tasks_fs::FileSystemPath;
use turbopack_core::{
    asset::Asset,
    output::{OptionOutputAsset, OutputAsset, OutputAssets},
    source_map::{GenerateSourceMap, OptionSourceMap},
    version::OptionVersionedContent,
};

/// An unresolved output assets operation. We need to pass an operation here as
/// it's stored for later usage and we want to reconnect this operation when
/// it's received from the map again.
#[turbo_tasks::value(transparent)]
pub struct OutputAssetsOperation(Vc<OutputAssets>);

#[derive(Clone, TraceRawVcs, PartialEq, Eq, ValueDebugFormat, Serialize, Deserialize, Debug)]
struct MapEntry {
    assets_operation: Vc<OutputAssets>,
    side_effects: Vc<Completion>,
    /// Precomputed map for quick access to output asset by filepath
    path_to_asset: HashMap<Vc<FileSystemPath>, Vc<Box<dyn OutputAsset>>>,
}

#[turbo_tasks::value(transparent)]
struct OptionMapEntry(Option<MapEntry>);

type PathToOutputOperation = HashMap<Vc<FileSystemPath>, FxIndexSet<Vc<OutputAssets>>>;
// A precomputed map for quick access to output asset by filepath
type OutputOperationToComputeEntry = HashMap<Vc<OutputAssets>, Vc<OptionMapEntry>>;

#[turbo_tasks::value]
pub struct VersionedContentMap {
    // TODO: turn into a bi-directional multimap, OutputAssets -> FxIndexSet<FileSystemPath>
    map_path_to_op: State<PathToOutputOperation>,
    map_op_to_compute_entry: State<OutputOperationToComputeEntry>,
}

impl ValueDefault for VersionedContentMap {
    fn value_default() -> Vc<Self> {
        VersionedContentMap {
            map_path_to_op: State::new(HashMap::new()),
            map_op_to_compute_entry: State::new(HashMap::new()),
        }
        .cell()
    }
}

impl VersionedContentMap {
    // NOTE(alexkirsz) This must not be a `#[turbo_tasks::function]` because it
    // should be a singleton for each project.
    pub fn new() -> Vc<Self> {
        Self::value_default()
    }
}

#[turbo_tasks::value_impl]
impl VersionedContentMap {
    /// Inserts output assets into the map and returns a completion that when
    /// awaited will emit the assets that were inserted.
    #[turbo_tasks::function]
    pub async fn insert_output_assets(
        self: Vc<Self>,
        // Output assets to emit
        assets_operation: Vc<OutputAssetsOperation>,
        node_root: Vc<FileSystemPath>,
        client_relative_path: Vc<FileSystemPath>,
        client_output_path: Vc<FileSystemPath>,
    ) -> Result<Vc<Completion>> {
        let this = self.await?;
        let compute_entry = self.compute_entry(
            assets_operation,
            node_root,
            client_relative_path,
            client_output_path,
        );
        let assets = *assets_operation.await?;
        this.map_op_to_compute_entry
            .update_conditionally(|map| map.insert(assets, compute_entry) != Some(compute_entry));
        let Some(entry) = &*compute_entry.await? else {
            unreachable!("compute_entry always returns Some(MapEntry)")
        };
        Ok(entry.side_effects)
    }

    /// Creates a ComputEntry (a pre-computed map for optimized lookup) for an output assets
    /// operation. When assets change, map_path_to_op is updated.
    #[turbo_tasks::function]
    async fn compute_entry(
        &self,
        assets_operation: Vc<OutputAssetsOperation>,
        node_root: Vc<FileSystemPath>,
        client_relative_path: Vc<FileSystemPath>,
        client_output_path: Vc<FileSystemPath>,
    ) -> Result<Vc<OptionMapEntry>> {
        let assets = *assets_operation.await?;
        async fn get_entries(
            assets: Vc<OutputAssets>,
        ) -> Result<Vec<(Vc<FileSystemPath>, Vc<Box<dyn OutputAsset>>)>> {
            let assets_ref = assets.await?;
            let entries = assets_ref
                .iter()
                .map(|&asset| async move {
                    let path = asset.ident().path().resolve().await?;
                    Ok((path, asset))
                })
                .try_join()
                .await?;
            Ok(entries)
        }
        let entries = get_entries(assets).await.unwrap_or_default();

        self.map_path_to_op.update_conditionally(|map| {
            let mut changed = false;

            // get current map's keys, subtract keys that don't exist in operation
            let mut stale_assets = map.keys().copied().collect::<HashSet<_>>();

            for (k, _) in entries.iter() {
                let res = map.entry(*k).or_default().insert(assets);
                stale_assets.remove(k);
                changed = changed || res;
            }

            // Make more efficient with reverse map
            for k in &stale_assets {
                let res = map
                    .get_mut(k)
                    // guaranteed
                    .unwrap()
                    .remove(&assets);
                changed = changed || res
            }
            changed
        });

        // Make sure all written client assets are up-to-date
        let side_effects = emit_assets(assets, node_root, client_relative_path, client_output_path);
        let map_entry = Vc::cell(Some(MapEntry {
            assets_operation: assets,
            side_effects,
            path_to_asset: entries.into_iter().collect(),
        }));
        Ok(map_entry)
    }

    #[turbo_tasks::function]
    pub async fn get(
        self: Vc<Self>,
        path: Vc<FileSystemPath>,
    ) -> Result<Vc<OptionVersionedContent>> {
        Ok(Vc::cell(
            (*self.get_asset(path).await?).map(|a| a.versioned_content()),
        ))
    }

    #[turbo_tasks::function]
    pub async fn get_source_map(
        self: Vc<Self>,
        path: Vc<FileSystemPath>,
        section: Option<RcStr>,
    ) -> Result<Vc<OptionSourceMap>> {
        let Some(asset) = &*self.get_asset(path).await? else {
            return Ok(Vc::cell(None));
        };

        if let Some(generate_source_map) =
            Vc::try_resolve_sidecast::<Box<dyn GenerateSourceMap>>(*asset).await?
        {
            Ok(if let Some(section) = section {
                generate_source_map.by_section(section)
            } else {
                generate_source_map.generate_source_map()
            })
        } else {
            let path = path.to_string().await?;
            bail!("no source map for path {}", path);
        }
    }

    #[turbo_tasks::function]
    pub async fn get_asset(
        self: Vc<Self>,
        path: Vc<FileSystemPath>,
    ) -> Result<Vc<OptionOutputAsset>> {
        let result = self.raw_get(path).await?;
        if let Some(MapEntry {
            assets_operation: _,
            side_effects,
            path_to_asset,
        }) = &*result
        {
            side_effects.await?;

            if let Some(asset) = path_to_asset.get(&path) {
                return Ok(Vc::cell(Some(*asset)));
            } else {
                let path = path.to_string().await?;
                bail!(
                    "could not find asset for path {} (asset has been removed)",
                    path,
                );
            }
        }

        Ok(Vc::cell(None))
    }

    #[turbo_tasks::function]
    pub async fn keys_in_path(&self, root: Vc<FileSystemPath>) -> Result<Vc<Vec<RcStr>>> {
        let keys = {
            let map = self.map_path_to_op.get();
            map.keys().copied().collect::<Vec<_>>()
        };
        let root = &root.await?;
        let keys = keys
            .into_iter()
            .map(|path| async move { Ok(root.get_path_to(&*path.await?).map(RcStr::from)) })
            .try_flat_join()
            .await?;
        Ok(Vc::cell(keys))
    }

    #[turbo_tasks::function]
    fn raw_get(&self, path: Vc<FileSystemPath>) -> Vc<OptionMapEntry> {
        let assets = {
            let map = self.map_path_to_op.get();
            map.get(&path).and_then(|m| m.iter().last().copied())
        };
        let Some(assets) = assets else {
            return Vc::cell(None);
        };
        // Need to reconnect the operation to the map
        Vc::connect(assets);

        let compute_entry = {
            let map = self.map_op_to_compute_entry.get();
            map.get(&assets).copied()
        };
        let Some(compute_entry) = compute_entry else {
            return Vc::cell(None);
        };
        // Need to reconnect the operation to the map
        Vc::connect(compute_entry);

        compute_entry
    }
}
