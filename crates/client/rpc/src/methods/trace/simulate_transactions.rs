use blockifier::transaction::objects::TransactionExecutionInfo;
use jsonrpsee::core::RpcResult;
use mc_genesis_data_provider::GenesisProvider;
use mc_storage::StorageOverride;
use mp_hashers::HasherT;
use mp_simulations::{PlaceHolderErrorTypeForFailedStarknetExecution, SimulationFlags};
use mp_transactions::from_broadcasted_transactions::ToAccountTransaction;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use starknet_core::types::{
    BlockId, BroadcastedTransaction, FeeEstimate, PriceUnit, SimulatedTransaction, SimulationFlag,
};

use super::lib::ConvertCallInfoToExecuteInvocationError;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::Starknet;

pub async fn simulate_transactions<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
    transactions: Vec<BroadcastedTransaction>,
    simulation_flags: Vec<SimulationFlag>,
) -> RpcResult<Vec<SimulatedTransaction>>
where
    A: ChainApi<Block = DBlockT> + 'static,
    BE: Backend<DBlockT> + 'static,
    G: GenesisProvider + Send + Sync + 'static,
    C: HeaderBackend<DBlockT> + BlockBackend<DBlockT> + StorageProvider<DBlockT, BE> + 'static,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT> + ConvertTransactionRuntimeApi<DBlockT>,
    P: TransactionPool<Block = DBlockT> + 'static,
    H: HasherT + Send + Sync + 'static,
{
    let substrate_block_hash =
        starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|_e| StarknetRpcApiError::BlockNotFound)?;

    let tx_type_and_tx_iterator = transactions.into_iter().map(|tx| match tx {
        BroadcastedTransaction::Invoke(_) => tx.to_account_transaction().map(|tx| (TxType::Invoke, tx)),
        BroadcastedTransaction::Declare(_) => tx.to_account_transaction().map(|tx| (TxType::Declare, tx)),
        BroadcastedTransaction::DeployAccount(_) => tx.to_account_transaction().map(|tx| (TxType::DeployAccount, tx)),
    });
    let (tx_types, user_transactions) =
        itertools::process_results(tx_type_and_tx_iterator, |iter| iter.unzip::<_, _, Vec<_>, Vec<_>>()).map_err(
            |e| {
                log::error!("Failed to convert BroadcastedTransaction to UserTransaction: {e}");
                StarknetRpcApiError::InternalServerError
            },
        )?;

    let simulation_flags = SimulationFlags::from(simulation_flags);

    let res = starknet
        .client
        .runtime_api()
        .simulate_transactions(substrate_block_hash, user_transactions, simulation_flags)
        .map_err(|e| {
            log::error!("Request parameters error: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            log::error!("Failed to call function: {:#?}", e);
            StarknetRpcApiError::ContractError
        })?;

    let storage_override = starknet.overrides.for_block_hash(starknet.client.as_ref(), substrate_block_hash);
    let simulated_transactions =
        tx_execution_infos_to_simulated_transactions(&**storage_override, substrate_block_hash, tx_types, res)
            .map_err(StarknetRpcApiError::from)?;

    Ok(simulated_transactions)
}

fn tx_execution_infos_to_simulated_transactions<B: BlockT>(
    storage_override: &dyn StorageOverride<B>,
    substrate_block_hash: B::Hash,
    tx_types: Vec<TxType>,
    transaction_execution_results: Vec<
        Result<TransactionExecutionInfo, PlaceHolderErrorTypeForFailedStarknetExecution>,
    >,
) -> Result<Vec<SimulatedTransaction>, ConvertCallInfoToExecuteInvocationError> {
    let mut results = vec![];
    for (tx_type, res) in tx_types.into_iter().zip(transaction_execution_results.into_iter()) {
        match res {
            Ok(tx_exec_info) => {
                let transaction_trace =
                    tx_execution_infos_to_tx_trace(storage_override, substrate_block_hash, tx_type, &tx_exec_info)?;
                let gas = tx_exec_info.execute_call_info.as_ref().map(|x| x.execution.gas_consumed).unwrap_or_default();
                let fee = tx_exec_info.actual_fee.0;
                // TODO: Shouldn't the gas price be taken from the block header instead?
                let price = if gas > 0 { fee / gas as u128 } else { 0 };

                let gas_consumed = gas.into();
                let gas_price = price.into();
                let overall_fee = fee.into();

                let unit: PriceUnit = PriceUnit::Wei; //TODO(Tbelleng) : Get Price Unit from Tx
                let data_gas_consumed = tx_exec_info.da_gas.l1_data_gas.into();
                let data_gas_price = tx_exec_info.da_gas.l1_gas.into();

                results.push(SimulatedTransaction {
                    transaction_trace,
                    fee_estimation: FeeEstimate {
                        gas_consumed,
                        data_gas_consumed,
                        data_gas_price,
                        gas_price,
                        overall_fee,
                        unit,
                    },
                });
            }
            Err(_) => {
                return Err(ConvertCallInfoToExecuteInvocationError::TransactionExecutionFailed);
            }
        }
    }

    Ok(results)
}
