use starknet_core::types::{BlockId, MaybePendingBlockWithTxHashes};

use crate::errors::StarknetRpcResult;
use dp_block::DeoxysMaybePendingBlockInfo;
use starknet_core::types::{BlockStatus, BlockWithTxHashes, PendingBlockWithTxHashes};

use crate::Starknet;

/// Get block information with transaction hashes given the block id.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag.
///
/// ### Returns
///
/// Returns block information with transaction hashes. This includes either a confirmed block or
/// a pending block with transaction hashes, depending on the state of the requested block.
/// In case the block is not found, returns a `StarknetRpcApiError` with `BlockNotFound`.

pub fn get_block_with_tx_hashes(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<MaybePendingBlockWithTxHashes> {
    let block = starknet.get_block_info(&block_id)?;

    let block_txs_hashes = block.tx_hashes().to_vec();

    match block {
        DeoxysMaybePendingBlockInfo::Pending(block) => {
            Ok(MaybePendingBlockWithTxHashes::PendingBlock(PendingBlockWithTxHashes {
                transactions: block_txs_hashes,
                parent_hash: block.header.parent_block_hash,
                timestamp: block.header.block_timestamp,
                sequencer_address: block.header.sequencer_address,
                l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                l1_da_mode: block.header.l1_da_mode.into(),
                starknet_version: block.header.protocol_version.to_string(),
            }))
        }
        DeoxysMaybePendingBlockInfo::NotPending(block) => {
            let status = if block.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            Ok(MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
                transactions: block_txs_hashes,
                status,
                block_hash: block.block_hash,
                parent_hash: block.header.parent_block_hash,
                block_number: block.header.block_number,
                new_root: block.header.global_state_root,
                timestamp: block.header.block_timestamp,
                sequencer_address: block.header.sequencer_address,
                l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                l1_da_mode: block.header.l1_da_mode.into(),
                starknet_version: block.header.protocol_version.to_string(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::StarknetRpcApiError,
        test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters},
    };
    use rstest::rstest;
    use starknet_core::types::{BlockTag, Felt, L1DataAvailabilityMode, ResourcePrice};

    #[rstest]
    fn test_get_block_with_tx_hashes(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { block_hashes, tx_hashes, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        let res = MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
            status: BlockStatus::AcceptedOnL1,
            block_hash: block_hashes[0],
            parent_hash: Felt::ZERO,
            block_number: 0,
            new_root: Felt::from_hex_unchecked("0x88912"),
            timestamp: 43,
            sequencer_address: Felt::from_hex_unchecked("0xbabaa"),
            l1_gas_price: ResourcePrice { price_in_fri: 12.into(), price_in_wei: 123.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 52.into(), price_in_wei: 44.into() },
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "0.13.1.1".into(),
            transactions: vec![tx_hashes[0]],
        });
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(0)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Hash(block_hashes[0])).unwrap(), res);

        // Block 1
        let res = MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
            status: BlockStatus::AcceptedOnL2,
            block_hash: block_hashes[1],
            parent_hash: block_hashes[0],
            block_number: 1,
            new_root: Felt::ZERO,
            timestamp: 0,
            sequencer_address: Felt::ZERO,
            l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: "0.13.2".into(),
            transactions: vec![],
        });
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(1)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Hash(block_hashes[1])).unwrap(), res);

        // Block 2
        let res = MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
            status: BlockStatus::AcceptedOnL2,
            block_hash: block_hashes[2],
            parent_hash: block_hashes[1],
            block_number: 2,
            new_root: Felt::ZERO,
            timestamp: 0,
            sequencer_address: Felt::ZERO,
            l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "0.13.2".into(),
            transactions: vec![tx_hashes[1], tx_hashes[2]],
        });
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Tag(BlockTag::Latest)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(2)).unwrap(), res);
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Hash(block_hashes[2])).unwrap(), res);

        // Pending
        let res = MaybePendingBlockWithTxHashes::PendingBlock(PendingBlockWithTxHashes {
            parent_hash: block_hashes[2],
            timestamp: 0,
            sequencer_address: Felt::ZERO,
            l1_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_data_gas_price: ResourcePrice { price_in_fri: 0.into(), price_in_wei: 0.into() },
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "0.13.2".into(),
            transactions: vec![tx_hashes[3]],
        });
        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Tag(BlockTag::Pending)).unwrap(), res);
    }

    #[rstest]
    fn test_get_block_with_tx_hashes_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        assert_eq!(get_block_with_tx_hashes(&rpc, BlockId::Number(3)), Err(StarknetRpcApiError::BlockNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(
            get_block_with_tx_hashes(&rpc, BlockId::Hash(does_not_exist)),
            Err(StarknetRpcApiError::BlockNotFound)
        );
    }
}