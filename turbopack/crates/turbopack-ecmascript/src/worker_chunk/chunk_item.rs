use anyhow::{bail, Result};
use indoc::formatdoc;
use turbo_tasks::{RcStr, TryJoinIterExt, Value, ValueToString, Vc};
use turbopack_core::{
    chunk::{
        availability_info::AvailabilityInfo, ChunkData, ChunkItem, ChunkType, ChunkingContext,
        ChunkingContextExt, ChunksData, EvaluatableAsset, EvaluatableAssets,
    },
    ident::AssetIdent,
    module::Module,
    output::OutputAssets,
    reference::{ModuleReferences, SingleOutputAssetReference},
};

use super::module::WorkerLoaderModule;
use crate::{
    chunk::{
        data::EcmascriptChunkData, EcmascriptChunkItem, EcmascriptChunkItemContent,
        EcmascriptChunkType,
    },
    utils::StringifyJs,
};

#[turbo_tasks::value(shared)]
pub struct WorkerLoaderChunkItem {
    pub module: Vc<WorkerLoaderModule>,
    pub chunking_context: Vc<Box<dyn ChunkingContext>>,
}

#[turbo_tasks::function]
pub fn worker_modifier() -> Vc<RcStr> {
    Vc::cell("worker".into())
}

#[turbo_tasks::value_impl]
impl WorkerLoaderChunkItem {
    #[turbo_tasks::function]
    async fn chunks(self: Vc<Self>) -> Result<Vc<OutputAssets>> {
        let this = self.await?;
        let module = this.module.await?;

        let Some(evaluatable) =
            Vc::try_resolve_downcast::<Box<dyn EvaluatableAsset>>(module.inner).await?
        else {
            bail!(
                "{} is not evaluatable for Worker loader module",
                module.inner.ident().to_string().await?
            );
        };

        Ok(this.chunking_context.evaluated_chunk_group_assets(
            AssetIdent::from_path(
                this.chunking_context
                    .chunk_path(module.inner.ident(), ".js".into()),
            )
            .with_modifier(worker_modifier()),
            EvaluatableAssets::empty().with_entry(evaluatable),
            Value::new(AvailabilityInfo::Root),
        ))
    }

    #[turbo_tasks::function]
    async fn chunks_data(self: Vc<Self>) -> Result<Vc<ChunksData>> {
        let this = self.await?;
        Ok(ChunkData::from_assets(
            this.chunking_context.output_root(),
            self.chunks(),
        ))
    }
}

#[turbo_tasks::value_impl]
impl EcmascriptChunkItem for WorkerLoaderChunkItem {
    #[turbo_tasks::function]
    fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
        self.chunking_context
    }

    #[turbo_tasks::function]
    async fn content(self: Vc<Self>) -> Result<Vc<EcmascriptChunkItemContent>> {
        let chunks_data = self.chunks_data().await?;
        let chunks_data = chunks_data.iter().try_join().await?;
        let chunks_data: Vec<_> = chunks_data
            .iter()
            .map(|chunk_data| EcmascriptChunkData::new(chunk_data))
            .collect();

        let code = formatdoc! {
            r#"
                __turbopack_export_value__(__turbopack_worker_blob_url__({chunks:#}));
            "#,
            chunks = StringifyJs(&chunks_data),
        };

        Ok(EcmascriptChunkItemContent {
            inner_code: code.into(),
            ..Default::default()
        }
        .into())
    }
}

#[turbo_tasks::function]
fn chunk_reference_description() -> Vc<RcStr> {
    Vc::cell("worker chunk".into())
}

#[turbo_tasks::value_impl]
impl ChunkItem for WorkerLoaderChunkItem {
    #[turbo_tasks::function]
    fn asset_ident(&self) -> Vc<AssetIdent> {
        self.module.ident()
    }

    #[turbo_tasks::function]
    async fn content_ident(&self) -> Result<Vc<AssetIdent>> {
        Ok(self.module.ident())
    }

    #[turbo_tasks::function]
    async fn references(self: Vc<Self>) -> Result<Vc<ModuleReferences>> {
        let chunks = self.chunks();

        Ok(Vc::cell(
            chunks
                .await?
                .iter()
                .copied()
                .map(|chunk| {
                    Vc::upcast(SingleOutputAssetReference::new(
                        chunk,
                        chunk_reference_description(),
                    ))
                })
                .collect(),
        ))
    }

    #[turbo_tasks::function]
    async fn chunking_context(&self) -> Vc<Box<dyn ChunkingContext>> {
        Vc::upcast(self.chunking_context)
    }

    #[turbo_tasks::function]
    async fn ty(&self) -> Result<Vc<Box<dyn ChunkType>>> {
        Ok(Vc::upcast(
            Vc::<EcmascriptChunkType>::default().resolve().await?,
        ))
    }

    #[turbo_tasks::function]
    fn module(&self) -> Vc<Box<dyn Module>> {
        Vc::upcast(self.module)
    }
}
