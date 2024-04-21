//! Converts types from [`starknet_providers`] to madara's expected types.

use std::collections::HashMap;
use std::num::NonZeroU128;
use std::sync::Arc;

use blockifier::blockifier::block::GasPrices;
use mp_block::DeoxysBlock;
use mp_felt::Felt252Wrapper;
use starknet_api::hash::StarkFelt;
use starknet_api::transaction::{
    DeclareTransaction, DeployAccountTransaction, DeployAccountTransactionV1, DeployTransaction, Event,
    InvokeTransaction, L1HandlerTransaction, Transaction,
};
use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, PendingStateUpdate,
    ReplacedClassItem, StateDiff as StateDiffCore, StorageEntry,
};
use starknet_ff::FieldElement;
use starknet_providers::sequencer::models::state_update::{
    DeclaredContract, DeployedContract, StateDiff as StateDiffProvider, StorageDiff as StorageDiffProvider,
};
use starknet_providers::sequencer::models::{self as p, StateUpdate as StateUpdateProvider};

use crate::commitments::lib::calculate_commitments;
use crate::utility::get_config;

pub async fn block(block: p::Block) -> DeoxysBlock {
    tokio::task::spawn_blocking(|| convert_block_sync(block)).await.expect("join error")
}

pub fn convert_block_sync(block: p::Block) -> DeoxysBlock {
    // converts starknet_provider transactions and events to mp_transactions and starknet_api events
    let transactions = transactions(block.transactions);
    let events = events(&block.transaction_receipts);
    let parent_block_hash = felt(block.parent_block_hash);
    let block_number = block.block_number.expect("no block number provided");
    let block_timestamp = block.timestamp;
    let global_state_root = felt(block.state_root.expect("no state root provided"));
    let sequencer_address = block.sequencer_address.map_or(contract_address(FieldElement::ZERO), contract_address);
    let transaction_count = transactions.len() as u128;
    let event_count = events.len() as u128;

    let (transaction_commitment, event_commitment) = commitments(&transactions, &events, block_number);

    let protocol_version = starknet_version(&block.starknet_version);
    let l1_gas_price = resource_price(block.l1_gas_price, block.l1_data_gas_price);
    let l1_da_mode = l1_da_mode(block.l1_da_mode);
    let extra_data = block.block_hash.map(|h| sp_core::U256::from_big_endian(&h.to_bytes_be()));

    let header = mp_block::Header {
        parent_block_hash,
        block_number,
        block_timestamp,
        global_state_root,
        sequencer_address,
        transaction_count,
        transaction_commitment,
        event_count,
        event_commitment,
        protocol_version,
        l1_gas_price,
        l1_da_mode,
        extra_data,
    };

    let ordered_events: Vec<mp_block::OrderedEvents> = block
        .transaction_receipts
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.events.is_empty())
        .map(|(i, r)| mp_block::OrderedEvents::new(i as u128, r.events.iter().map(event).collect()))
        .collect();

    DeoxysBlock::new(header, transactions, ordered_events)
}

fn transactions(txs: Vec<p::TransactionType>) -> Vec<Transaction> {
    txs.into_iter().map(transaction).collect()
}

fn transaction(transaction: p::TransactionType) -> Transaction {
    match transaction {
        p::TransactionType::Declare(tx) => Transaction::Declare(declare_transaction(tx)),
        p::TransactionType::Deploy(tx) => Transaction::Deploy(deploy_transaction(tx)),
        p::TransactionType::DeployAccount(tx) => Transaction::DeployAccount(deploy_account_transaction(tx)),
        p::TransactionType::InvokeFunction(tx) => Transaction::Invoke(invoke_transaction(tx)),
        p::TransactionType::L1Handler(tx) => Transaction::L1Handler(l1_handler_transaction(tx)),
    }
}

fn declare_transaction(tx: p::DeclareTransaction) -> DeclareTransaction {
    if tx.version == FieldElement::ZERO || tx.version == FieldElement::ONE {
        DeclareTransaction::V1(starknet_api::transaction::DeclareTransactionV0V1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            sender_address: contract_address(tx.sender_address),
        })
    } else if tx.version == FieldElement::TWO {
        DeclareTransaction::V2(starknet_api::transaction::DeclareTransactionV2 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            compiled_class_hash: compiled_class_hash(tx.compiled_class_hash.expect("no compiled class hash provided")),
            sender_address: contract_address(tx.sender_address),
        })
    } else if tx.version == FieldElement::THREE {
        DeclareTransaction::V3(starknet_api::transaction::DeclareTransactionV3 {
            resource_bounds: resource_bounds(tx.resource_bounds.expect("no resource bounds provided")),
            tip: tip(tx.tip.expect("no tip provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            compiled_class_hash: compiled_class_hash(tx.compiled_class_hash.expect("no compiled class hash provided")),
            sender_address: contract_address(tx.sender_address),
            nonce_data_availability_mode: data_availability_mode(
                tx.nonce_data_availability_mode.expect("no nonce_data_availability_mode provided"),
            ),
            fee_data_availability_mode: data_availability_mode(
                tx.fee_data_availability_mode.expect("no fee_data_availability_mode provided"),
            ),
            paymaster_data: paymaster_data(tx.paymaster_data.expect("no paymaster_data provided")),
            account_deployment_data: account_deployment_data(
                tx.account_deployment_data.expect("no account_deployment_data provided"),
            ),
        })
    } else {
        panic!("declare transaction version not supported");
    }
}

fn deploy_transaction(tx: p::DeployTransaction) -> DeployTransaction {
    DeployTransaction {
        version: transaction_version(tx.version),
        class_hash: class_hash(tx.class_hash),
        contract_address_salt: contract_address_salt(tx.contract_address_salt),
        constructor_calldata: call_data(tx.constructor_calldata),
    }
}

fn deploy_account_transaction(tx: p::DeployAccountTransaction) -> DeployAccountTransaction {
    match deploy_account_transaction_version(&tx) {
        1 => DeployAccountTransaction::V1(DeployAccountTransactionV1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            contract_address_salt: contract_address_salt(tx.contract_address_salt),
            constructor_calldata: call_data(tx.constructor_calldata),
        }),

        3 => DeployAccountTransaction::V3(starknet_api::transaction::DeployAccountTransactionV3 {
            resource_bounds: resource_bounds(tx.resource_bounds.expect("no resource bounds provided")),
            tip: tip(tx.tip.expect("no tip provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce),
            class_hash: class_hash(tx.class_hash),
            contract_address_salt: contract_address_salt(tx.contract_address_salt),
            constructor_calldata: call_data(tx.constructor_calldata),
            nonce_data_availability_mode: data_availability_mode(
                tx.nonce_data_availability_mode.expect("no nonce_data_availability_mode provided"),
            ),
            fee_data_availability_mode: data_availability_mode(
                tx.fee_data_availability_mode.expect("no fee_data_availability_mode provided"),
            ),
            paymaster_data: paymaster_data(tx.paymaster_data.expect("no paymaster_data provided")),
        }),

        _ => panic!("deploy account transaction version not supported"),
    }
}

// TODO: implement something better than this
fn deploy_account_transaction_version(tx: &p::DeployAccountTransaction) -> u8 {
    if tx.resource_bounds.is_some() { 3 } else { 1 }
}

fn invoke_transaction(tx: p::InvokeFunctionTransaction) -> InvokeTransaction {
    if tx.version == FieldElement::ZERO {
        InvokeTransaction::V0(starknet_api::transaction::InvokeTransactionV0 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            contract_address: contract_address(tx.sender_address),
            entry_point_selector: entry_point(tx.entry_point_selector.expect("no entry_point_selector provided")),
            calldata: call_data(tx.calldata),
        })
    } else if tx.version == FieldElement::ONE {
        InvokeTransaction::V1(starknet_api::transaction::InvokeTransactionV1 {
            max_fee: fee(tx.max_fee.expect("no max fee provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce.expect("no nonce provided")),
            sender_address: contract_address(tx.sender_address),
            calldata: call_data(tx.calldata),
        })
    } else if tx.version == FieldElement::THREE {
        InvokeTransaction::V3(starknet_api::transaction::InvokeTransactionV3 {
            resource_bounds: resource_bounds(tx.resource_bounds.expect("no resource bounds provided")),
            tip: tip(tx.tip.expect("no tip provided")),
            signature: signature(tx.signature),
            nonce: nonce(tx.nonce.expect("no nonce provided")),
            sender_address: contract_address(tx.sender_address),
            calldata: call_data(tx.calldata),
            nonce_data_availability_mode: data_availability_mode(
                tx.nonce_data_availability_mode.expect("no nonce_data_availability_mode provided"),
            ),
            fee_data_availability_mode: data_availability_mode(
                tx.fee_data_availability_mode.expect("no fee_data_availability_mode provided"),
            ),
            paymaster_data: paymaster_data(tx.paymaster_data.expect("no paymaster_data provided")),
            account_deployment_data: account_deployment_data(
                tx.account_deployment_data.expect("no account_deployment_data provided"),
            ),
        })
    } else {
        panic!("invoke transaction version not supported");
    }
}

fn l1_handler_transaction(tx: p::L1HandlerTransaction) -> L1HandlerTransaction {
    L1HandlerTransaction {
        version: transaction_version(tx.version),
        nonce: nonce(tx.nonce.unwrap_or_default()), // TODO check when a L1Ha
        contract_address: contract_address(tx.contract_address),
        entry_point_selector: entry_point(tx.entry_point_selector),
        calldata: call_data(tx.calldata),
    }
}

/// Converts a starknet version string to a felt value.
/// If the string contains more than 31 bytes, the function panics.
fn starknet_version(version: &Option<String>) -> Felt252Wrapper {
    match version {
        Some(version) => {
            Felt252Wrapper::try_from(version.as_bytes()).expect("Failed to convert version to felt: string is too long")
        }
        None => Felt252Wrapper::ZERO,
    }
}

fn fee(felt: starknet_ff::FieldElement) -> starknet_api::transaction::Fee {
    starknet_api::transaction::Fee(felt.try_into().expect("Value out of range for u128"))
}

fn signature(signature: Vec<starknet_ff::FieldElement>) -> starknet_api::transaction::TransactionSignature {
    starknet_api::transaction::TransactionSignature(signature.into_iter().map(felt).collect())
}

fn contract_address(address: starknet_ff::FieldElement) -> starknet_api::core::ContractAddress {
    starknet_api::core::ContractAddress(starknet_api::core::PatriciaKey(felt(address)))
}

fn entry_point(entry_point: starknet_ff::FieldElement) -> starknet_api::core::EntryPointSelector {
    starknet_api::core::EntryPointSelector(felt(entry_point))
}

fn call_data(call_data: Vec<starknet_ff::FieldElement>) -> starknet_api::transaction::Calldata {
    starknet_api::transaction::Calldata(Arc::new(call_data.into_iter().map(felt).collect()))
}

// TODO: is this function needed?
#[allow(dead_code)]
fn tx_hash(tx_hash: starknet_ff::FieldElement) -> starknet_api::transaction::TransactionHash {
    starknet_api::transaction::TransactionHash(felt(tx_hash))
}

fn nonce(nonce: starknet_ff::FieldElement) -> starknet_api::core::Nonce {
    starknet_api::core::Nonce(felt(nonce))
}

fn class_hash(class_hash: starknet_ff::FieldElement) -> starknet_api::core::ClassHash {
    starknet_api::core::ClassHash(felt(class_hash))
}

fn compiled_class_hash(compiled_class_hash: starknet_ff::FieldElement) -> starknet_api::core::CompiledClassHash {
    starknet_api::core::CompiledClassHash(felt(compiled_class_hash))
}

fn contract_address_salt(
    contract_address_salt: starknet_ff::FieldElement,
) -> starknet_api::transaction::ContractAddressSalt {
    starknet_api::transaction::ContractAddressSalt(felt(contract_address_salt))
}

fn transaction_version(version: starknet_ff::FieldElement) -> starknet_api::transaction::TransactionVersion {
    starknet_api::transaction::TransactionVersion(felt(version))
}

fn resource_bounds(
    ressource_bounds: starknet_providers::sequencer::models::ResourceBoundsMapping,
) -> starknet_api::transaction::ResourceBoundsMapping {
    starknet_api::transaction::ResourceBoundsMapping::try_from(vec![
        (
            starknet_api::transaction::Resource::L1Gas,
            starknet_api::transaction::ResourceBounds {
                max_amount: ressource_bounds.l1_gas.max_amount,
                max_price_per_unit: ressource_bounds.l1_gas.max_price_per_unit,
            },
        ),
        (
            starknet_api::transaction::Resource::L2Gas,
            starknet_api::transaction::ResourceBounds {
                max_amount: ressource_bounds.l2_gas.max_amount,
                max_price_per_unit: ressource_bounds.l2_gas.max_price_per_unit,
            },
        ),
    ])
    .expect("Failed to convert resource bounds")
}

fn tip(tip: u64) -> starknet_api::transaction::Tip {
    starknet_api::transaction::Tip(tip)
}

fn data_availability_mode(
    mode: starknet_providers::sequencer::models::DataAvailabilityMode,
) -> starknet_api::data_availability::DataAvailabilityMode {
    match mode {
        starknet_providers::sequencer::models::DataAvailabilityMode::L1 => {
            starknet_api::data_availability::DataAvailabilityMode::L1
        }
        starknet_providers::sequencer::models::DataAvailabilityMode::L2 => {
            starknet_api::data_availability::DataAvailabilityMode::L2
        }
    }
}

fn paymaster_data(paymaster_data: Vec<FieldElement>) -> starknet_api::transaction::PaymasterData {
    starknet_api::transaction::PaymasterData(paymaster_data.into_iter().map(felt).collect())
}

fn account_deployment_data(
    account_deployment_data: Vec<FieldElement>,
) -> starknet_api::transaction::AccountDeploymentData {
    starknet_api::transaction::AccountDeploymentData(account_deployment_data.into_iter().map(felt).collect())
}

/// Converts the l1 gas price and l1 data gas price to a GasPrices struct, if the l1 gas price is
/// not 0. If the l1 gas price is 0, returns None.
/// The other prices are converted to NonZeroU128, with 0 being converted to 1.
fn resource_price(
    l1_gas_price: starknet_core::types::ResourcePrice,
    l1_data_gas_price: starknet_core::types::ResourcePrice,
) -> Option<GasPrices> {
    /// Converts a FieldElement to a NonZeroU128, with 0 being converted to 1.
    fn field_element_to_non_zero_u128(field_element: FieldElement) -> NonZeroU128 {
        let value: u128 = if field_element == FieldElement::ZERO {
            1
        } else {
            field_element.try_into().expect("FieldElement is more than u128")
        };
        NonZeroU128::new(value).expect("Failed to convert field_element to NonZeroU128")
    }

    if l1_gas_price.price_in_wei == FieldElement::ZERO {
        None
    } else {
        Some(GasPrices {
            eth_l1_gas_price: field_element_to_non_zero_u128(l1_gas_price.price_in_wei),
            strk_l1_gas_price: field_element_to_non_zero_u128(l1_gas_price.price_in_fri),
            eth_l1_data_gas_price: field_element_to_non_zero_u128(l1_data_gas_price.price_in_wei),
            strk_l1_data_gas_price: field_element_to_non_zero_u128(l1_data_gas_price.price_in_fri),
        })
    }
}

fn l1_da_mode(
    mode: starknet_core::types::L1DataAvailabilityMode,
) -> starknet_api::data_availability::L1DataAvailabilityMode {
    match mode {
        starknet_core::types::L1DataAvailabilityMode::Calldata => {
            starknet_api::data_availability::L1DataAvailabilityMode::Calldata
        }
        starknet_core::types::L1DataAvailabilityMode::Blob => {
            starknet_api::data_availability::L1DataAvailabilityMode::Blob
        }
    }
}

fn events(receipts: &[p::ConfirmedTransactionReceipt]) -> Vec<starknet_api::transaction::Event> {
    receipts.iter().flat_map(|r| &r.events).map(event).collect()
}

fn event(event: &p::Event) -> starknet_api::transaction::Event {
    use starknet_api::transaction::{EventContent, EventData, EventKey};

    Event {
        from_address: contract_address(event.from_address),
        content: EventContent {
            keys: event.keys.iter().copied().map(felt).map(EventKey).collect(),
            data: EventData(event.data.iter().copied().map(felt).collect()),
        },
    }
}

fn commitments(
    transactions: &[starknet_api::transaction::Transaction],
    events: &[starknet_api::transaction::Event],
    block_number: u64,
) -> (StarkFelt, StarkFelt) {
    let chain_id = chain_id();

    let (commitment_tx, commitment_event) = calculate_commitments(transactions, events, chain_id, block_number);

    (commitment_tx.into(), commitment_event.into())
}

fn chain_id() -> mp_felt::Felt252Wrapper {
    match get_config() {
        Ok(config) => config.chain_id.into(),
        Err(e) => {
            log::error!("Failed to get chain id: {}", e);
            FieldElement::from_byte_slice_be(b"").unwrap().into()
        }
    }
}

fn felt(field_element: starknet_ff::FieldElement) -> starknet_api::hash::StarkFelt {
    starknet_api::hash::StarkFelt::new(field_element.to_bytes_be()).unwrap()
}

pub fn state_update(state_update: StateUpdateProvider) -> PendingStateUpdate {
    let old_root = state_update.old_root;
    let state_diff = state_diff(state_update.state_diff);

    // StateUpdateCore { block_hash, old_root, new_root, state_diff }
    PendingStateUpdate { old_root, state_diff }
}

fn state_diff(state_diff: StateDiffProvider) -> StateDiffCore {
    let storage_diffs = storage_diffs(state_diff.storage_diffs);
    let deprecated_declared_classes = state_diff.old_declared_contracts;
    let declared_classes = declared_classes(state_diff.declared_classes);
    let deployed_contracts = deployed_contracts(state_diff.deployed_contracts);
    let replaced_classes = replaced_classes(state_diff.replaced_classes);
    let nonces = nonces(state_diff.nonces);

    StateDiffCore {
        storage_diffs,
        deprecated_declared_classes,
        declared_classes,
        deployed_contracts,
        replaced_classes,
        nonces,
    }
}

fn storage_diffs(storage_diffs: HashMap<FieldElement, Vec<StorageDiffProvider>>) -> Vec<ContractStorageDiffItem> {
    storage_diffs
        .into_iter()
        .map(|(address, entries)| ContractStorageDiffItem { address, storage_entries: storage_entries(entries) })
        .collect()
}

fn storage_entries(storage_entries: Vec<StorageDiffProvider>) -> Vec<StorageEntry> {
    storage_entries.into_iter().map(|StorageDiffProvider { key, value }| StorageEntry { key, value }).collect()
}

fn declared_classes(declared_classes: Vec<DeclaredContract>) -> Vec<DeclaredClassItem> {
    declared_classes
        .into_iter()
        .map(|DeclaredContract { class_hash, compiled_class_hash }| DeclaredClassItem {
            class_hash,
            compiled_class_hash,
        })
        .collect()
}

fn deployed_contracts(deplyed_contracts: Vec<DeployedContract>) -> Vec<DeployedContractItem> {
    deplyed_contracts
        .into_iter()
        .map(|DeployedContract { address, class_hash }| DeployedContractItem { address, class_hash })
        .collect()
}

fn replaced_classes(replaced_classes: Vec<DeployedContract>) -> Vec<ReplacedClassItem> {
    replaced_classes
        .into_iter()
        .map(|DeployedContract { address, class_hash }| ReplacedClassItem { contract_address: address, class_hash })
        .collect()
}

fn nonces(nonces: HashMap<FieldElement, FieldElement>) -> Vec<NonceUpdate> {
    // TODO: make sure the order is `contract_address` -> `nonce`
    // and not `nonce` -> `contract_address`
    nonces.into_iter().map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce }).collect()
}
