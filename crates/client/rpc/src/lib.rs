//! Starknet RPC server API implementation
//!
//! It uses the deoxys client and backend in order to answer queries.

mod constants;
mod errors;
pub mod providers;
#[cfg(test)]
pub mod test_utils;
mod types;
pub mod utils;
pub mod versions;

use anyhow::bail;
use jsonrpsee::RpcModule;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

use dc_db::db_block_id::DbBlockIdResolvable;
use dc_db::DeoxysBackend;
use dp_block::{DeoxysMaybePendingBlock, DeoxysMaybePendingBlockInfo};
use dp_chain_config::{ChainConfig, RpcVersion};
use dp_convert::ToFelt;

use errors::{StarknetRpcApiError, StarknetRpcResult};
use providers::AddTransactionProvider;
use utils::ResultExt;
use versions::v0_7_1;

/// A Starknet RPC server for Deoxys
#[derive(Clone)]
pub struct Starknet {
    backend: Arc<DeoxysBackend>,
    chain_config: Arc<ChainConfig>,
    pub(crate) add_transaction_provider: Arc<dyn AddTransactionProvider>,
}

impl Starknet {
    pub fn new(
        backend: Arc<DeoxysBackend>,
        chain_config: Arc<ChainConfig>,
        add_transaction_provider: Arc<dyn AddTransactionProvider>,
    ) -> Self {
        Self { backend, add_transaction_provider, chain_config }
    }

    pub fn clone_backend(&self) -> Arc<DeoxysBackend> {
        Arc::clone(&self.backend)
    }

    pub fn get_block_info(
        &self,
        block_id: &impl DbBlockIdResolvable,
    ) -> StarknetRpcResult<DeoxysMaybePendingBlockInfo> {
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

    pub fn get_block(&self, block_id: &impl DbBlockIdResolvable) -> StarknetRpcResult<DeoxysMaybePendingBlock> {
        self.backend
            .get_block(block_id)
            .or_internal_server_error("Error getting block from storage")?
            .ok_or(StarknetRpcApiError::BlockNotFound)
    }

    pub fn chain_id(&self) -> Felt {
        self.chain_config.chain_id.clone().to_felt()
    }

    pub fn current_block_number(&self) -> StarknetRpcResult<u64> {
        self.get_block_n(&dp_block::BlockId::Tag(dp_block::BlockTag::Latest))
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

pub fn versioned_rpc_api(starknet: &Starknet, read: bool, write: bool, trace: bool) -> anyhow::Result<RpcModule<()>> {
    let mut rpc_api = RpcModule::new(());

    // TODO: Any better way to do that?
    for rpc_version in dp_chain_config::SUPPORTED_RPC_VERSIONS.iter() {
        match *rpc_version {
            RpcVersion::RPC_VERSION_0_7_1 => {
                if read {
                    rpc_api.merge(v0_7_1::StarknetReadRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
                }
                if write {
                    rpc_api.merge(v0_7_1::StarknetWriteRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
                }
                if trace {
                    rpc_api.merge(v0_7_1::StarknetTraceRpcApiV0_7_1Server::into_rpc(starknet.clone()))?;
                }
            }
            _ => bail!("Unrecognized RPC spec version: {} - check the [SUPPORTED_RPC_VERSIONS] constant.", rpc_version),
        }
    }
    Ok(rpc_api)
}
