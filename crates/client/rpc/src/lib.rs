//! Starknet RPC server API implementation
//!
//! It uses the madara client and backend in order to answer queries.

mod constants;
mod errors;
pub mod providers;
#[cfg(test)]
pub mod test_utils;
mod types;
pub mod utils;
pub mod versions;

use jsonrpsee::RpcModule;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

use mc_db::db_block_id::DbBlockIdResolvable;
use mc_db::MadaraBackend;
use mp_block::{BlockId, BlockTag, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo};
use mp_chain_config::{ChainConfig, RpcVersion};
use mp_convert::ToFelt;

pub use errors::{StarknetRpcApiError, StarknetRpcResult};
use providers::AddTransactionProvider;
use utils::ResultExt;

/// A Starknet RPC server for Madara
#[derive(Clone)]
pub struct Starknet {
    backend: Arc<MadaraBackend>,
    pub(crate) add_transaction_provider: Arc<dyn AddTransactionProvider>,
}

impl Starknet {
    pub fn new(backend: Arc<MadaraBackend>, add_transaction_provider: Arc<dyn AddTransactionProvider>) -> Self {
        Self { backend, add_transaction_provider }
    }

    pub fn clone_backend(&self) -> Arc<MadaraBackend> {
        Arc::clone(&self.backend)
    }

    pub fn clone_chain_config(&self) -> Arc<ChainConfig> {
        Arc::clone(self.backend.chain_config())
    }

    pub fn get_block_info(
        &self,
        block_id: &impl DbBlockIdResolvable,
    ) -> StarknetRpcResult<MadaraMaybePendingBlockInfo> {
        self.backend
            .get_block_info(block_id)
            .or_internal_server_error("Error getting block from storage")?
            .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    pub fn get_block_n(&self, block_id: &impl DbBlockIdResolvable) -> StarknetRpcResult<u64> {
        self.backend
            .get_block_n(block_id)
            .or_internal_server_error("Error getting block from storage")?
            .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    pub fn get_block(&self, block_id: &impl DbBlockIdResolvable) -> StarknetRpcResult<MadaraMaybePendingBlock> {
        self.backend
            .get_block(block_id)
            .or_internal_server_error("Error getting block from storage")?
            .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    pub fn chain_id(&self) -> Felt {
        self.backend.chain_config().chain_id.clone().to_felt()
    }

    pub fn current_block_number(&self) -> StarknetRpcResult<u64> {
        self.get_block_n(&BlockId::Tag(BlockTag::Latest))
    }

    pub fn current_spec_version(&self) -> RpcVersion {
        RpcVersion::RPC_VERSION_LATEST
    }

    pub fn get_l1_last_confirmed_block(&self) -> StarknetRpcResult<u64> {
        Ok(self
            .backend
            .get_l1_last_confirmed_block()
            .or_internal_server_error("Error getting L1 last confirmed block")?
            .unwrap_or_default())
    }
}

/// Returns the RpcModule merged with all the supported RPC versions.
pub fn versioned_rpc_api(
    starknet: &Starknet,
    read: bool,
    write: bool,
    trace: bool,
    internal: bool,
    ws: bool,
) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    if read {
        rpc_api.merge(versions::v0_7_1::StarknetReadRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
        rpc_api.merge(versions::v0_8_0::StarknetReadRpcApiV0_8_0Server::into_rpc(starknet.clone()))?;
    }
    if write {
        rpc_api.merge(versions::v0_7_1::StarknetWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    }
    if trace {
        rpc_api.merge(versions::v0_7_1::StarknetTraceRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    }
    if internal {
        rpc_api.merge(versions::v0_7_1::MadaraWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
    }
    if ws {
        rpc_api.merge(versions::v0_8_0::StarknetWsRpcApiV0_8_0Server::into_rpc(starknet.clone()))?;
    }

    Ok(rpc_api)
}
