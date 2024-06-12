use std::sync::Arc;

use anyhow::Context;
use dc_db::{DatabaseService, DeoxysBackend};
use dc_metrics::MetricsRegistry;
use dc_sync::fetch::fetchers::FetchConfig;
use dc_sync::metrics::block_metrics::BlockMetrics;
use dc_telemetry::TelemetryHandle;
use primitive_types::H160;
use starknet_core::types::FieldElement;
use tokio::task::JoinSet;
use url::Url;

use crate::cli::SyncParams;

#[derive(Clone)]
pub struct SyncService {
    db_backend: Arc<DeoxysBackend>,
    fetch_config: FetchConfig,
    backup_every_n_blocks: Option<u64>,
    l1_endpoint: Url,
    l1_core_address: H160,
    starting_block: Option<u64>,
    block_metrics: BlockMetrics,
    chain_id: FieldElement,
    start_params: Option<TelemetryHandle>,
}

impl SyncService {
    pub fn new(
        config: &SyncParams,
        db: &DatabaseService,
        metrics_handle: MetricsRegistry,
        telemetry: TelemetryHandle,
    ) -> anyhow::Result<Self> {
        let block_metrics = BlockMetrics::register(&metrics_handle)?;
        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            fetch_config: config.block_fetch_config(),
            l1_endpoint: config.l1_endpoint.clone(),
            l1_core_address: config.network.l1_core_address(),
            starting_block: config.starting_block,
            backup_every_n_blocks: config.backup_every_n_blocks,
            block_metrics,
            chain_id: config.network.chain_id(),
            start_params: Some(telemetry),
        })
    }
    pub async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        let SyncService {
            fetch_config,
            backup_every_n_blocks,
            l1_endpoint,
            l1_core_address,
            starting_block,
            block_metrics,
            chain_id,
            ..
        } = self.clone();
        let telemetry = self.start_params.take().context("service already started")?;

        let db_backend = Arc::clone(&self.db_backend);
        join_set.spawn(async move {
            dc_sync::starknet_sync_worker::sync(
                &db_backend,
                fetch_config,
                l1_endpoint,
                l1_core_address,
                starting_block,
                backup_every_n_blocks,
                block_metrics,
                chain_id,
                telemetry,
            )
            .await
        });

        Ok(())
    }
}
