use std::sync::Arc;

use mc_db::{DatabaseService, MadaraBackend};
use mc_mempool::{block_production::BlockProductionTask, L1DataProvider, Mempool};
use mc_metrics::MetricsRegistry;
use mc_telemetry::TelemetryHandle;
use mp_utils::service::Service;
use tokio::task::JoinSet;

use crate::cli::block_production::BlockProductionParams;

struct StartParams {
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    l1_data_provider: Arc<dyn L1DataProvider>,
}

pub struct BlockProductionService {
    start: Option<StartParams>,
    enabled: bool,
}
impl BlockProductionService {
    pub fn new(
        config: &BlockProductionParams,
        db_service: &DatabaseService,
        mempool: Arc<mc_mempool::Mempool>,
        l1_data_provider: Arc<dyn L1DataProvider>,
        _metrics_handle: MetricsRegistry,
        _telemetry: TelemetryHandle,
    ) -> anyhow::Result<Self> {
        if config.block_production_disabled {
            return Ok(Self { start: None, enabled: false });
        }

        Ok(Self {
            start: Some(StartParams { backend: Arc::clone(db_service.backend()), l1_data_provider, mempool }),
            enabled: true,
        })
    }
}

#[async_trait::async_trait]
impl Service for BlockProductionService {
    // TODO(cchudant,30-07-2024): special threading requirements for the block production task
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let StartParams { backend, l1_data_provider, mempool } = self.start.take().expect("Service already started");

        join_set.spawn(async move {
            BlockProductionTask::new(backend, mempool, l1_data_provider)?.block_production_task().await?;
            Ok(())
        });

        Ok(())
    }
}
