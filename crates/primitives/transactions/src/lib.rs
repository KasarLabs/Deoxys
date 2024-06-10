//! Starknet transaction related functionality.
#![cfg_attr(not(feature = "std"), no_std)]

#[doc(hidden)]
pub extern crate alloc;

pub mod compute_hash;
#[cfg(feature = "client")]
pub mod from_broadcasted_transactions;
pub mod getters;
#[cfg(feature = "client")]
pub mod to_starknet_core_transaction;
#[cfg(feature = "client")]
pub mod utils;

use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transaction_types::TransactionType;

use starknet_types_core::felt::Felt;

const SIMULATE_TX_VERSION_OFFSET: Felt =
    Felt::from_raw([18446744073700081665, 17407, 18446744073709551584, 576460752142434320]);
// old implementation as a field : FieldElement::from_mont([18446744073700081665, 17407, 18446744073709551584, 576460752142434320]);

/// Legacy check for deprecated txs
/// See `https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/` for more details.

pub const LEGACY_BLOCK_NUMBER: u64 = 1470;
pub const LEGACY_L1_HANDLER_BLOCK: u64 = 854;

/// Wrapper type for transaction execution error.
/// Different tx types.
/// See `https://docs.starknet.io/documentation/architecture_and_concepts/Blocks/transactions/` for more details.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode, parity_scale_codec::Decode))]
#[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
pub enum TxType {
    /// Regular invoke transaction.
    Invoke,
    /// Declare transaction.
    Declare,
    /// Deploy account transaction.
    DeployAccount,
    /// Message sent from ethereum.
    L1Handler,
}

impl From<TxType> for TransactionType {
    fn from(value: TxType) -> Self {
        match value {
            TxType::Invoke => TransactionType::InvokeFunction,
            TxType::Declare => TransactionType::Declare,
            TxType::DeployAccount => TransactionType::DeployAccount,
            TxType::L1Handler => TransactionType::L1Handler,
        }
    }
}

impl From<&Transaction> for TxType {
    fn from(value: &Transaction) -> Self {
        match value {
            Transaction::AccountTransaction(tx) => tx.into(),
            Transaction::L1HandlerTransaction(_) => TxType::L1Handler,
        }
    }
}

impl From<&AccountTransaction> for TxType {
    fn from(value: &AccountTransaction) -> Self {
        match value {
            AccountTransaction::Declare(_) => TxType::Declare,
            AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
            AccountTransaction::Invoke(_) => TxType::Invoke,
        }
    }
}

// impl From<&UserTransaction> for TxType {
//     fn from(value: &UserTransaction) -> Self {
//         match value {
//             UserTransaction::Declare(_, _) => TxType::Declare,
//             UserTransaction::DeployAccount(_) => TxType::DeployAccount,
//             UserTransaction::Invoke(_) => TxType::Invoke,
//         }
//     }
// }

// impl From<&UserOrL1HandlerTransaction> for TxType {
//     fn from(value: &UserOrL1HandlerTransaction) -> Self {
//         match value {
//             UserOrL1HandlerTransaction::User(tx) => tx.into(),
//             UserOrL1HandlerTransaction::L1Handler(_, _) => TxType::L1Handler,
//         }
//     }
// }

// #[derive(Clone, Debug, Eq, PartialEq, From)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub enum UserTransaction {
//     Declare(DeclareTransaction),
//     DeployAccount(DeployAccountTransaction),
//     Invoke(InvokeTransaction),
// }

// #[derive(Clone, Debug, Eq, PartialEq, From, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub enum Transaction {
//     Declare(DeclareTransaction),
//     DeployAccount(DeployAccountTransaction),
//     Deploy(DeployTransaction),
//     Invoke(InvokeTransaction),
//     L1Handler(HandleL1MessageTransaction),
// }

// #[derive(Clone, Debug, Eq, PartialEq, From)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub enum UserOrL1HandlerTransaction {
//     User(AccountTransaction),
//     L1Handler(L1HandlerTransaction),
// }

// #[derive(Debug, Clone, Eq, PartialEq, From, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub enum InvokeTransaction {
//     V0(InvokeTransactionV0),
//     V1(InvokeTransactionV1),
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct InvokeTransactionV0 {
//     pub max_fee: u128,
//     pub signature: Vec<Felt252Wrapper>,
//     pub contract_address: Felt252Wrapper,
//     pub entry_point_selector: Felt252Wrapper,
//     pub calldata: Vec<Felt252Wrapper>,
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct InvokeTransactionV1 {
//     pub max_fee: u128,
//     pub signature: Vec<Felt252Wrapper>,
//     pub nonce: Felt252Wrapper,
//     pub sender_address: Felt252Wrapper,
//     pub calldata: Vec<Felt252Wrapper>,
//     pub offset_version: bool,
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub enum DeclareTransaction {
//     V0(DeclareTransactionV0V1),
//     V1(DeclareTransactionV0V1),
//     V2(DeclareTransactionV2),
//     V3(DeclareTransactionV3),
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct DeclareTransactionV0V1 {
//     pub max_fee: u128,
//     pub signature: Vec<Felt252Wrapper>,
//     pub nonce: Felt252Wrapper,
//     pub class_hash: Felt252Wrapper,
//     pub sender_address: Felt252Wrapper,
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct DeclareTransactionV2 {
//     pub max_fee: u128,
//     pub signature: Vec<Felt252Wrapper>,
//     pub nonce: Felt252Wrapper,
//     pub class_hash: Felt252Wrapper,
//     pub compiled_class_hash: Felt252Wrapper,
//     pub sender_address: Felt252Wrapper,
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct DeclareTransactionV3 {
//     pub resource_bounds: ResourceBoundsMapping,
//     pub tip: u64,
//     pub signature: Vec<Felt252Wrapper>,
//     pub nonce: Felt252Wrapper,
//     pub class_hash: Felt252Wrapper,
//     pub compiled_class_hash: Felt252Wrapper,
//     pub sender_address: Felt252Wrapper,
//     pub nonce_data_availability_mode: DataAvailabilityMode,
//     pub fee_data_availability_mode: DataAvailabilityMode,
//     pub paymaster_data: PaymasterData,
//     pub account_deployment_data: AccountDeploymentData,
// }

// #[derive(Debug, Clone, Eq, PartialEq, From, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub enum DeployAccountTransaction {
//     V1(DeployAccountTransactionV1),
//     V3(DeployAccountTransactionV3),
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct DeployAccountTransactionV1 {
//     pub max_fee: u128,
//     pub signature: Vec<Felt252Wrapper>,
//     pub nonce: Felt252Wrapper,
//     pub class_hash: Felt252Wrapper,
//     pub contract_address_salt: Felt252Wrapper,
//     pub constructor_calldata: Vec<Felt252Wrapper>,
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct DeployAccountTransactionV3 {
//     pub resource_bounds: ResourceBoundsMapping,
//     pub tip: u64,
//     pub signature: Vec<Felt252Wrapper>,
//     pub nonce: Felt252Wrapper,
//     pub class_hash: Felt252Wrapper,
//     pub contract_address_salt: Felt252Wrapper,
//     pub constructor_calldata: Vec<Felt252Wrapper>,
//     pub nonce_data_availability_mode: DataAvailabilityMode,
//     pub fee_data_availability_mode: DataAvailabilityMode,
//     pub paymaster_data: Vec<Felt252Wrapper>,
//     pub max_fee: u128,
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct DeployTransaction {
//     pub version: TransactionVersion,
//     pub class_hash: Felt252Wrapper,
//     pub contract_address: Felt252Wrapper,
//     pub contract_address_salt: Felt252Wrapper,
//     pub constructor_calldata: Vec<Felt252Wrapper>,
// }

// #[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
// #[cfg_attr(feature = "parity-scale-codec", derive(parity_scale_codec::Encode,
// parity_scale_codec::Decode))] #[cfg_attr(feature = "scale-info", derive(scale_info::TypeInfo))]
// pub struct HandleL1MessageTransaction {
//     pub nonce: u64,
//     pub contract_address: Felt252Wrapper,
//     pub entry_point_selector: Felt252Wrapper,
//     pub calldata: Vec<Felt252Wrapper>,
// }

// impl From<UserTransaction> for AccountTransaction {
//     fn from(user_transaction: UserTransaction) -> Self {
//         match user_transaction {
//             UserTransaction::Declare(declare_tx) => AccountTransaction::Declare(declare_tx),
//             UserTransaction::DeployAccount(deploy_account_tx) =>
// AccountTransaction::DeployAccount(deploy_account_tx),
// UserTransaction::Invoke(invoke_tx) => AccountTransaction::Invoke(invoke_tx),         }
//     }
// }

// impl From<AccountTransaction> for UserTransaction {
//     fn from(account_transaction: AccountTransaction) -> Self {
//         match account_transaction {
//             AccountTransaction::Declare(declare_tx) => UserTransaction::Declare(declare_tx),
//             AccountTransaction::DeployAccount(deploy_account_tx) =>
// UserTransaction::DeployAccount(deploy_account_tx),
// AccountTransaction::Invoke(invoke_tx) => UserTransaction::Invoke(invoke_tx),         }
//     }
// }

// impl From<UserOrL1HandlerTransaction> for Transaction {
//     fn from(item: UserOrL1HandlerTransaction) -> Self {
//         match item {
//             UserOrL1HandlerTransaction::User(tx) => Transaction::AccountTransaction(tx),
//             UserOrL1HandlerTransaction::L1Handler(tx) => Transaction::L1HandlerTransaction(tx),
//         }
//     }
// }

// impl From<Transaction> for UserOrL1HandlerTransaction {
//     fn from(item: Transaction) -> Self {
//         match item {
//             Transaction::AccountTransaction(tx) => UserOrL1HandlerTransaction::User(tx),
//             Transaction::L1HandlerTransaction(tx) => UserOrL1HandlerTransaction::L1Handler(tx),
//         }
//     }
// }

// pub fn user_or_l1_into_tx_vec(user_or_l1_transactions: Vec<UserOrL1HandlerTransaction>) ->
// Vec<Transaction> {     user_or_l1_transactions.into_iter().map(|tx| tx.into()).collect()
// }

// pub fn tx_into_user_or_l1_vec(transactions: Vec<Transaction>) -> Vec<UserOrL1HandlerTransaction>
// {     transactions.into_iter().map(|tx| tx.into()).collect()
// }
