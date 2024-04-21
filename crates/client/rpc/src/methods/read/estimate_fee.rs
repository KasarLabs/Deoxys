use blockifier::transaction::account_transaction::AccountTransaction;
use jsonrpsee::core::RpcResult;
use mc_genesis_data_provider::GenesisProvider;
use mp_hashers::HasherT;
use mp_simulations::convert_flags;
use mp_transactions::from_broadcasted_transactions::ToAccountTransaction;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{
    BlockId, BroadcastedTransaction, FeeEstimate, SimulationFlagForEstimateFee as EstimateFeeFlag,
};

use crate::errors::StarknetRpcApiError;
use crate::Starknet;

/// Estimate the fee associated with transaction
///
/// # Arguments
///
/// * `request` - starknet transaction request
/// * `block_id` - hash of the requested block, number (height), or tag
///
/// # Returns
///
/// * `fee_estimate` - fee estimate in gwei
pub async fn estimate_fee<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    request: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<EstimateFeeFlag>,
    block_id: BlockId,
) -> RpcResult<Vec<FeeEstimate>>
where
    A: ChainApi<Block = DBlockT> + 'static,
    P: TransactionPool<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    G: GenesisProvider + Send + Sync + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        log::error!("'{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let transactions = request
        .into_iter()
        .map(|tx| tx.to_account_transaction())
        .collect::<Result<Vec<AccountTransaction>, _>>()
        .map_err(|e| {
            log::error!("Failed to convert BroadcastedTransaction to AccountTransaction: {e}");
            StarknetRpcApiError::InternalServerError
        })?;

    let account_transactions: Vec<AccountTransaction> =
        transactions.into_iter().map(AccountTransaction::from).collect();

    let simulation_flags = convert_flags(simulation_flags);

    let fee_estimates = starknet
        .client
        .runtime_api()
        .estimate_fee(substrate_block_hash, account_transactions, simulation_flags)
        .map_err(|e| {
            log::error!("Request parameters error: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            log::error!("Failed to call function: {:#?}", e);
            StarknetRpcApiError::ContractError
        })?;

    let estimates = fee_estimates
            .into_iter()
			// FIXME: https://github.com/keep-starknet-strange/madara/issues/329
            // TODO: reflect right estimation
            .map(|x| FeeEstimate { gas_consumed: x.gas_consumed.0 , gas_price: x.gas_price.0, data_gas_consumed: x.data_gas_consumed.0, data_gas_price: x.data_gas_price.0, overall_fee: x.overall_fee.0, unit: x.unit.into()})
            .collect();

    Ok(estimates)
}
