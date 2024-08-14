use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

/// Get the value of the storage at the given address and key.
///
/// This function retrieves the value stored in a specified contract's storage, identified by a
/// contract address and a storage key, within a specified block in the current network.
///
/// ### Arguments
///
/// * `contract_address` - The address of the contract to read from. This parameter identifies the
///   contract whose storage is being queried.
/// * `key` - The key to the storage value for the given contract. This parameter specifies the
///   particular storage slot to be queried.
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter defines the state of the blockchain at which the storage value is to
///   be read.
///
/// ### Returns
///
/// Returns the value at the given key for the given contract, represented as a `Felt`.
/// If no value is found at the specified storage key, returns 0.
///
/// ### Errors
///
/// This function may return errors in the following cases:
///
/// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
/// * `CONTRACT_NOT_FOUND` - If the specified contract does not exist or is not deployed at the
///   given `contract_address` in the specified block.
pub fn get_storage_at(
    starknet: &Starknet,
    contract_address: Felt,
    key: Felt,
    block_id: BlockId,
) -> StarknetRpcResult<Felt> {
    // Check if block exists. We have to return a different error in that case.
    let block_exists =
        starknet.backend.contains_block(&block_id).or_internal_server_error("Checking if block is in database")?;
    if !block_exists {
        return Err(StarknetRpcApiError::BlockNotFound);
    }

    // Check if contract exists
    starknet
        .backend
        .get_contract_class_hash_at(&block_id, &contract_address) // TODO: contains api without deser
        .or_internal_server_error("Failed to check if contract is deployed")?
        .ok_or(StarknetRpcApiError::ContractNotFound)?;

    let storage = starknet
        .backend
        .get_contract_storage_at(&block_id, &contract_address, &key)
        .or_internal_server_error("Error getting contract class hash at")?
        .unwrap_or(Felt::ZERO);

    Ok(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_state_updates, SampleChainForStateUpdates};
    use rstest::rstest;
    use starknet_core::types::BlockTag;

    #[rstest]
    fn test_get_storage_at(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { keys, values, contracts, .. }, rpc) = sample_chain_for_state_updates;

        // Expected values are in the format `values[contract][key] = value`.
        let check_contract_key_value = |block_n, contracts_kv: [Option<[Felt; 3]>; 3]| {
            for (contract_i, contract_values) in contracts_kv.into_iter().enumerate() {
                if let Some(contract_values) = contract_values {
                    for (key_i, value) in contract_values.into_iter().enumerate() {
                        assert_eq!(
                            get_storage_at(&rpc, contracts[contract_i], keys[key_i], block_n).unwrap(),
                            value,
                            "get storage at blockid {block_n:?}, contract #{contract_i}, key #{key_i}"
                        );
                    }
                } else {
                    // contract not found
                    for (key_i, _) in keys.iter().enumerate() {
                        assert_eq!(
                            get_storage_at(&rpc, contracts[contract_i], keys[key_i], block_n),
                            Err(StarknetRpcApiError::ContractNotFound),
                            "get storage at blockid {block_n:?}, contract #{contract_i}, key #{key_i} should not found"
                        );
                    }
                }
            }
        };

        // The SampleChainForStateUpdates has a few storage changes happening on keys[0..3] on contracts[0..3],
        // for each block, we check all of the keys in all of the contracts

        // Block 0
        let block_n = BlockId::Number(0);
        let expected = [
            // contract[0] values for keys[0..3]. Second key is not found, which means Felt::ZERO.
            Some([values[0], Felt::ZERO, values[2]]),
            // contract[1] not deployed yet
            None,
            // contract[2] not deployed yet
            None,
        ];
        check_contract_key_value(block_n, expected);

        // Block 1
        let block_n = BlockId::Number(1);
        let expected = [
            Some([values[1], Felt::ZERO, values[2]]),
            Some([Felt::ZERO, Felt::ZERO, Felt::ZERO]),
            Some([Felt::ZERO, Felt::ZERO, values[0]]),
        ];
        check_contract_key_value(block_n, expected);

        // Block 2
        let block_n = BlockId::Number(2);
        let expected = [
            Some([values[1], Felt::ZERO, values[2]]),
            Some([values[0], Felt::ZERO, Felt::ZERO]),
            Some([Felt::ZERO, values[2], values[0]]),
        ];
        check_contract_key_value(block_n, expected);

        // Pending
        let block_n = BlockId::Tag(BlockTag::Pending);
        let expected = [
            Some([values[2], values[0], values[2]]),
            Some([values[0], Felt::ZERO, Felt::ZERO]),
            Some([Felt::ZERO, values[2], values[0]]),
        ];
        check_contract_key_value(block_n, expected);
    }

    /// Checks BlockNotFound, ContractNotFound and key not found cases.
    #[rstest]
    fn test_get_storage_at_not_found(sample_chain_for_state_updates: (SampleChainForStateUpdates, Starknet)) {
        let (SampleChainForStateUpdates { keys, contracts, .. }, rpc) = sample_chain_for_state_updates;

        // Not found
        let block_n = BlockId::Number(3);
        assert_eq!(get_storage_at(&rpc, contracts[0], keys[0], block_n), Err(StarknetRpcApiError::BlockNotFound));
        let block_n = BlockId::Number(0);
        assert_eq!(get_storage_at(&rpc, contracts[1], keys[0], block_n), Err(StarknetRpcApiError::ContractNotFound));
        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_storage_at(&rpc, does_not_exist, keys[0], block_n), Err(StarknetRpcApiError::ContractNotFound));
        assert_eq!(
            get_storage_at(&rpc, contracts[0], keys[1], block_n),
            Ok(Felt::ZERO) // return ZERO when key not found
        );
    }
}
