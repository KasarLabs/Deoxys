use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction,
    DeclareTransactionResult, DeployAccountTransactionResult, InvokeTransactionResult,
};
use starknet_providers::{Provider, ProviderError};
use mp_transactions::BroadcastedDeclareTransactionV0;
use crate::{bail_internal_server_error, errors::StarknetRpcApiError};

use super::AddTransactionProvider;

pub struct ForwardToProvider<P: Provider + Send + Sync> {
    provider: P,
}

impl<P: Provider + Send + Sync> ForwardToProvider<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl<P: Provider + Send + Sync> AddTransactionProvider for ForwardToProvider<P> {

    async fn add_declare_v0_transaction(&self, declare_v0_transaction: BroadcastedDeclareTransactionV0) -> RpcResult<DeclareTransactionResult> {
        // panic here, because we can't really forward it to the real FGW, or shall we enable it so that another madara full node is able to use it?
        // maybe a flag for this? as discussed
        unimplemented!()
    }
    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTransaction,
    ) -> RpcResult<DeclareTransactionResult> {
        let sequencer_response = match self.provider.add_declare_transaction(declare_transaction).await {
            Ok(response) => response,
            Err(ProviderError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add declare transaction to sequencer: {e}"),
        };

        Ok(sequencer_response)
    }
    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTransaction,
    ) -> RpcResult<DeployAccountTransactionResult> {
        let sequencer_response = match self.provider.add_deploy_account_transaction(deploy_account_transaction).await {
            Ok(response) => response,
            Err(ProviderError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add deploy account transaction to sequencer: {e}"),
        };

        Ok(sequencer_response)
    }

    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTransaction,
    ) -> RpcResult<InvokeTransactionResult> {
        let sequencer_response = match self.provider.add_invoke_transaction(invoke_transaction).await {
            Ok(response) => response,
            Err(ProviderError::StarknetError(e)) => {
                return Err(StarknetRpcApiError::from(e).into());
            }
            Err(e) => bail_internal_server_error!("Failed to add invoke transaction to sequencer: {e}"),
        };

        Ok(sequencer_response)
    }
}
