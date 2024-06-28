use dc_exec::block_context;
use dc_exec::execute_transactions;
use dc_exec::execution_result_to_tx_trace;
use dp_block::StarknetVersion;
use dp_convert::ToStarkFelt;
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::Felt;
use starknet_core::types::TransactionTraceWithHash;

use crate::errors::StarknetRpcApiError;
use crate::utils::transaction::to_blockifier_transactions;
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;

// For now, we fallback to the sequencer - that is what pathfinder and juno do too, but this is temporary
pub const FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW: StarknetVersion = StarknetVersion::STARKNET_VERSION_0_13_1_1;

pub async fn trace_transaction(starknet: &Starknet, transaction_hash: Felt) -> RpcResult<TransactionTraceWithHash> {
    let (block, tx_info) = starknet
        .block_storage()
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error while getting block from tx hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_index = tx_info.tx_index;

    if block.header().protocol_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion.into());
    }

    let block_context = block_context(block.header(), &starknet.chain_id()).map_err(|e| {
        log::error!("Failed to create block context: {e}");
        StarknetRpcApiError::InternalServerError
    })?;

    // create a vector of tuples with the transaction and its hash, up to the current transaction index
    let mut transactions_before: Vec<_> = block
        .transactions()
        .iter()
        .zip(block.tx_hashes())
        .take(tx_index) // takes up until not including last tx
        .map(|(tx, hash)| to_blockifier_transactions(starknet, tx, &TransactionHash(hash.to_stark_felt())))
        .collect::<Result<_, _>>()?;

    let to_trace = transactions_before
        .pop()
        .ok_or_internal_server_error("Error: there should be at least one transaction in the block")?;

    let mut executions_results =
        execute_transactions(starknet.clone_backend(), transactions_before, [to_trace], &block_context, true, true)
            .or_internal_server_error("Failed to re-execute transactions")?;
    let execution_result =
        executions_results.pop().ok_or_internal_server_error("No execution info returned for the last transaction")?;

    let trace = execution_result_to_tx_trace(&execution_result)
        .or_internal_server_error("Converting execution infos to tx trace")?;

    let tx_trace = TransactionTraceWithHash { transaction_hash, trace_root: trace };

    Ok(tx_trace)
}
