use jsonrpsee::core::RpcResult;
use mc_genesis_data_provider::GenesisProvider;
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_transactions::to_starknet_core_transaction::to_starknet_core_tx;
use mp_types::block::DBlockT;
use pallet_starknet_runtime_api::{ConvertTransactionRuntimeApi, StarknetRuntimeApi};
use sc_client_api::backend::{Backend, StorageProvider};
use sc_client_api::BlockBackend;
use sc_transaction_pool::ChainApi;
use sc_transaction_pool_api::TransactionPool;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use starknet_core::types::{BlockId, BlockWithReceipts, MaybePendingBlockWithReceipts, TransactionWithReceipt};

use super::get_transaction_receipt::get_transaction_receipt_finalized;
use crate::errors::StarknetRpcApiError;
use crate::utils::block::{
    l1_da_mode, l1_data_gas_price, l1_gas_price, new_root, parent_hash, sequencer_address, starknet_version, timestamp,
};
use crate::utils::utils::status;
use crate::{get_block_by_block_hash, Starknet};

// TODO: Implem pending block
pub fn get_block_with_receipts<A, BE, G, C, P, H>(
    starknet: &Starknet<A, BE, G, C, P, H>,
    block_id: BlockId,
) -> RpcResult<MaybePendingBlockWithReceipts>
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

    let starknet_block = get_block_by_block_hash(starknet.client.as_ref(), substrate_block_hash).map_err(|e| {
        log::error!("Failed to get block for block hash {substrate_block_hash}: '{e}'");
        StarknetRpcApiError::InternalServerError
    })?;
    let block_hash = starknet_block.header().hash::<H>();
    let chain_id = starknet.chain_id()?;

    let transactions_with_receipts = starknet_block
        .transactions()
        .iter()
        .map(|tx| {
            let transaction_hash = tx.compute_hash::<H>(
                Felt252Wrapper::from(chain_id.0),
                false,
                Some(starknet_block.header().block_number),
            );
            let transaction = to_starknet_core_tx(tx.clone(), Felt252Wrapper::from(transaction_hash).into());
            let receipt_with_block_info = get_transaction_receipt_finalized(
                starknet,
                chain_id,
                substrate_block_hash,
                Felt252Wrapper::from(transaction_hash).into(),
            )
            .unwrap();

            let receipt = receipt_with_block_info.receipt;

            TransactionWithReceipt { transaction, receipt }
        })
        .collect::<Vec<TransactionWithReceipt>>();

    let status = status(starknet_block.header().block_number);
    let parent_hash = parent_hash(&starknet_block);
    let new_root = new_root(&starknet_block);
    let timestamp = timestamp(&starknet_block);
    let sequencer_address = sequencer_address(&starknet_block);
    let l1_gas_price = l1_gas_price(&starknet_block);
    let l1_data_gas_price = l1_data_gas_price(&starknet_block);
    let l1_da_mode = l1_da_mode(&starknet_block);
    let starknet_version = starknet_version(&starknet_block);

    let block_with_receipts = BlockWithReceipts {
        status,
        block_hash: block_hash.into(),
        parent_hash,
        block_number: starknet_block.header().block_number,
        new_root,
        timestamp,
        sequencer_address,
        l1_gas_price,
        l1_data_gas_price,
        l1_da_mode,
        starknet_version,
        transactions: transactions_with_receipts,
    };

    let block = MaybePendingBlockWithReceipts::Block(block_with_receipts);

    Ok(block)
}
