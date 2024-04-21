use std::sync::Arc;

use blockifier::state::cached_state::CommitmentStateDiff;
use indexmap::IndexMap;
use lazy_static::lazy_static;
use mc_db::storage_handler::{self, DeoxysStorageError, StorageView, StorageViewMut};
use mp_block::state_update::StateUpdateWrapper;
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use mp_hashers::poseidon::PoseidonHasher;
use mp_hashers::HasherT;
use mp_transactions::Transaction;
use rayon::prelude::*;
use starknet_api::api_core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_api::transaction::Event;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;

use super::events::memory_event_commitment;
use super::transactions::memory_transaction_commitment;

/// Calculate the transaction and event commitment.
///
/// # Arguments
///
/// * `transactions` - The transactions of the block
/// * `events` - The events of the block
/// * `chain_id` - The current chain id
/// * `block_number` - The current block number
///
/// # Returns
///
/// The transaction and the event commitment as `Felt252Wrapper`.
pub fn calculate_commitments(
    transactions: &[Transaction],
    events: &[Event],
    chain_id: Felt252Wrapper,
    block_number: u64,
) -> (Felt252Wrapper, Felt252Wrapper) {
    let (commitment_tx, commitment_event) = rayon::join(
        || memory_transaction_commitment(transactions, chain_id, block_number),
        || memory_event_commitment(events),
    );
    (
        commitment_tx.expect("Failed to calculate transaction commitment"),
        commitment_event.expect("Failed to calculate event commitment"),
    )
}

/// Builds a `CommitmentStateDiff` from the `StateUpdateWrapper`.
///
/// # Arguments
///
/// * `StateUpdateWrapper` - The last state update fetched and formated.
///
/// # Returns
///
/// The commitment state diff as a `CommitmentStateDiff`.
pub fn build_commitment_state_diff(state_update_wrapper: StateUpdateWrapper) -> CommitmentStateDiff {
    let mut commitment_state_diff = CommitmentStateDiff {
        address_to_class_hash: IndexMap::new(),
        address_to_nonce: IndexMap::new(),
        storage_updates: IndexMap::new(),
        class_hash_to_compiled_class_hash: IndexMap::new(),
    };

    for deployed_contract in state_update_wrapper.state_diff.deployed_contracts.iter() {
        let address = ContractAddress::from(deployed_contract.address);
        let class_hash = if address == ContractAddress::from(Felt252Wrapper::ONE) {
            // System contracts doesnt have class hashes
            ClassHash::from(Felt252Wrapper::ZERO)
        } else {
            ClassHash::from(deployed_contract.class_hash)
        };
        commitment_state_diff.address_to_class_hash.insert(address, class_hash);
    }

    for (address, nonce) in state_update_wrapper.state_diff.nonces.iter() {
        let contract_address = ContractAddress::from(*address);
        let nonce_value = Nonce::from(*nonce);
        commitment_state_diff.address_to_nonce.insert(contract_address, nonce_value);
    }

    for (address, storage_diffs) in state_update_wrapper.state_diff.storage_diffs.iter() {
        let contract_address = ContractAddress::from(*address);
        let mut storage_map = IndexMap::new();
        for storage_diff in storage_diffs.iter() {
            let key = StorageKey::from(storage_diff.key);
            let value = StarkFelt::from(storage_diff.value);
            storage_map.insert(key, value);
        }
        commitment_state_diff.storage_updates.insert(contract_address, storage_map);
    }

    for declared_class in state_update_wrapper.state_diff.declared_classes.iter() {
        let class_hash = ClassHash::from(declared_class.class_hash);
        let compiled_class_hash = CompiledClassHash::from(declared_class.compiled_class_hash);
        commitment_state_diff.class_hash_to_compiled_class_hash.insert(class_hash, compiled_class_hash);
    }

    commitment_state_diff
}

/// Calculate state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `contracts_trie_root` - The root of the contracts trie.
/// * `classes_trie_root` - The root of the classes trie.
///
/// # Returns
///
/// The state commitment as a `Felt252Wrapper`.
pub fn calculate_state_root<H: HasherT>(
    contracts_trie_root: Felt252Wrapper,
    classes_trie_root: Felt252Wrapper,
) -> Felt252Wrapper
where
    H: HasherT,
{
    let starknet_state_prefix = Felt252Wrapper::try_from("STARKNET_STATE_V0".as_bytes()).unwrap();

    if classes_trie_root == Felt252Wrapper::ZERO {
        contracts_trie_root
    } else {
        let state_commitment_hash =
            H::compute_hash_on_elements(&[starknet_state_prefix.0, contracts_trie_root.0, classes_trie_root.0]);

        state_commitment_hash.into()
    }
}

/// Update the state commitment hash value.
///
/// The state commitment is the digest that uniquely (up to hash collisions) encodes the state.
/// It combines the roots of two binary Merkle-Patricia tries of height 251 using Poseidon/Pedersen
/// hashers.
///
/// # Arguments
///
/// * `CommitmentStateDiff` - The commitment state diff inducing unprocessed state changes.
/// * `BonsaiDb` - The database responsible for storing computing the state tries.
///
/// # Returns
///
/// The updated state root as a `Felt252Wrapper`.
pub fn update_state_root(csd: CommitmentStateDiff, block_number: u64) -> Felt252Wrapper {
    // Update contract and its storage tries
    let (contract_trie_root, class_trie_root) = rayon::join(
        || contract_trie_root(&csd, block_number).expect("Failed to compute contract root"),
        || class_trie_root(&csd, block_number).expect("Failed to compute class root"),
    );

    calculate_state_root::<PoseidonHasher>(contract_trie_root, class_trie_root)
}

/// Calculates the contract trie root
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `bonsai_contract` - Bonsai db used to store contract hashes.
/// * `block_number`    - The current block number.
///
/// # Returns
///
/// The contract root.
fn contract_trie_root(csd: &CommitmentStateDiff, block_number: u64) -> Result<Felt252Wrapper, DeoxysStorageError> {
    // NOTE: handlers implicitely acquire a lock on their respective tries
    // for the duration of their livetimes
    let mut contract_write = storage_handler::contract_trie_mut()?;
    let mut storage_write = storage_handler::contract_storage_trie_mut()?;

    // First we insert the contract storage changes
    for (contract_address, updates) in csd.storage_updates.iter() {
        storage_write.init(contract_address)?;

        for (key, value) in updates {
            storage_write.insert(contract_address, key, *value)?;
        }
    }

    // Then we commit them
    storage_write.commit(block_number)?;

    // Then we compute the leaf hashes retrieving the corresponding storage root
    let storage_read = storage_handler::contract_storage_trie()?;
    let updates = csd
        .storage_updates
        .iter()
        .par_bridge()
        .map(|(contract_address, _)| {
            let storage_root = storage_read.root(contract_address).unwrap();
            let leaf_hash = contract_state_leaf_hash(csd, contract_address, storage_root, block_number);

            (contract_address, leaf_hash)
        })
        .collect::<Vec<_>>();

    // then we compute the contract root by applying the changes so far
    contract_write.update(updates)?;
    contract_write.commit(block_number)?;

    Ok(storage_handler::contract_trie()?.root()?.into())
}

fn contract_state_leaf_hash(
    csd: &CommitmentStateDiff,
    contract_address: &ContractAddress,
    storage_root: Felt,
    block_number: u64,
) -> Felt {
    let class_hash = class_hash(csd, contract_address, block_number);

    let storage_root = FieldElement::from_bytes_be(&storage_root.to_bytes_be()).unwrap();

    let nonce_bytes = csd.address_to_nonce.get(contract_address).unwrap_or(&Nonce::default()).0.0;
    let nonce = FieldElement::from_bytes_be(&nonce_bytes).unwrap();

    // computes the contract state leaf hash
    let contract_state_hash = PedersenHasher::hash_elements(class_hash, storage_root);
    let contract_state_hash = PedersenHasher::hash_elements(contract_state_hash, nonce);
    let contract_state_hash = PedersenHasher::hash_elements(contract_state_hash, FieldElement::ZERO);

    Felt::from_bytes_be(&contract_state_hash.to_bytes_be())
}

fn class_hash(csd: &CommitmentStateDiff, contract_address: &ContractAddress, block_number: u64) -> FieldElement {
    let class_hash = match csd.address_to_class_hash.get(contract_address) {
        Some(class_hash) => *class_hash,
        None => {
            let Ok(handler_class_hash) = storage_handler::class_hash() else {
                return FieldElement::ZERO;
            };
            let Ok(Some(class_hash)) = handler_class_hash.get_at(contract_address, block_number) else {
                return FieldElement::ZERO;
            };

            class_hash
        }
    };

    FieldElement::from_byte_slice_be(class_hash.0.bytes()).unwrap()
}

lazy_static! {
    static ref CONTRACT_CLASS_HASH_VERSION: FieldElement =
        FieldElement::from_byte_slice_be("CONTRACT_CLASS_LEAF_V0".as_bytes()).unwrap();
}

/// Calculates the class trie root
///
/// # Arguments
///
/// * `csd`          - Commitment state diff for the current block.
/// * `bonsai_class` - Bonsai db used to store class hashes.
/// * `block_number` - The current block number.
///
/// # Returns
///
/// The class root.
fn class_trie_root(csd: &CommitmentStateDiff, block_number: u64) -> Result<Felt252Wrapper, DeoxysStorageError> {
    let mut class_write = storage_handler::class_trie_mut()?;

    let updates = csd
        .class_hash_to_compiled_class_hash
        .iter()
        .par_bridge()
        .map(|(class_hash, compiled_class_hash)| {
            let compiled_class_hash = FieldElement::from_bytes_be(&compiled_class_hash.0.0).unwrap();

            let hash = PoseidonHasher::hash_elements(*CONTRACT_CLASS_HASH_VERSION, compiled_class_hash);

            (class_hash, hash)
        })
        .collect::<Vec<_>>();

    class_write.init()?;
    class_write.update(updates)?;
    class_write.commit(block_number)?;

    Ok(storage_handler::class_trie()?.root()?.into())
}
