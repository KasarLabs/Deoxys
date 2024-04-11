use jsonrpsee::core::RpcResult;
use mc_genesis_data_provider::GenesisProvider;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::getters::Hash;
use mp_transactions::TxType;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::{Backend, BlockBackend, StorageProvider};
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::utils::{
    get_previous_block_substrate_hash, map_transaction_to_user_transaction, tx_execution_infos_to_tx_trace,
};
use crate::errors::StarknetRpcApiError;
use crate::get_block_by_block_hash;
use crate::Starknet;

pub async fn trace_block_transactions<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
) -> RpcResult<Vec<TransactionTraceWithHash>>
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
    let substrate_block_hash = starknet.substrate_block_hash_from_starknet_block(block_id).map_err(|e| {
        log::error!("Block not found: '{e}'");
        StarknetRpcApiError::BlockNotFound
    })?;

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash).map_err(|e| {
        log::error!("Failed to get block for block hash {substrate_block_hash}: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let chain_id = Felt252Wrapper(starknet.chain_id()?.0);

    let (block_transactions, empty_transactions) =
        map_transaction_to_user_transaction(starknet, starknet_block, substrate_block_hash, chain_id, None)?;

    let previous_block_substrate_hash = get_previous_block_substrate_hash(starknet, substrate_block_hash)?;

    let fee_token_address = starknet.client.runtime_api().fee_token_addresses(substrate_block_hash).map_err(|e| {
        log::error!("Failed to retrieve fee token address: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash)?;
    let block_header = block.header();
    // TODO: convert the real chain_id in String
    let block_context =
        block_header.into_block_context(fee_token_address, starknet_api::core::ChainId("SN_MAIN".to_string()));

    let execution_infos = starknet
        .client
        .runtime_api()
        .re_execute_transactions(
            previous_block_substrate_hash,
            empty_transactions.clone(),
            block_transactions.clone(),
            &block_context,
        )
        .map_err(|e| {
            log::error!("Failed to execute runtime API call: {e}");
            StarknetRpcApiError::InternalServerError
        })?
        .map_err(|e| {
            log::error!("Failed to reexecute the block transactions: {e:?}");
            StarknetRpcApiError::InternalServerError
        })?;

    let storage_override = starknet.overrides.for_block_hash(starknet.client.as_ref(), substrate_block_hash);

    let traces = execution_infos
        .into_iter()
        .enumerate()
        .map(|(tx_idx, tx_exec_info)| {
            tx_execution_infos_to_tx_trace(
                &**storage_override,
                substrate_block_hash,
                // Safe to unwrap coz re_execute returns exactly one ExecutionInfo for each tx
                TxType::from(block_transactions.get(tx_idx).unwrap()),
                &tx_exec_info,
            )
            .map(|trace_root| TransactionTraceWithHash {
                transaction_hash: Felt252Wrapper::from(block_transactions[tx_idx].tx_hash().unwrap()).into(),
                trace_root,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(StarknetRpcApiError::from)?;

    Ok(traces)
}
