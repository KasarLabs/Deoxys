use std::fmt;
use std::path::PathBuf;
use std::string::String;
use std::vec::Vec;

use blockifier::execution::contract_class::ContractClass as StarknetContractClass;
use derive_more::Constructor;
use lazy_static::lazy_static;
use mp_felt::Felt252Wrapper;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::serde_as;
use starknet_core::serde::unsigned_field_element::UfeHex;
use starknet_crypto::FieldElement;

lazy_static! {
    static ref ETH_TOKEN_ADDR: HexFelt = HexFelt(
        FieldElement::from_hex_be("0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").unwrap()
    );
    static ref STRK_TOKEN_ADDR: HexFelt = HexFelt(
        FieldElement::from_hex_be("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d").unwrap()
    );
}

/// A wrapper for FieldElement that implements serde's Serialize and Deserialize for hex strings.
#[serde_as]
#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct HexFelt(#[serde_as(as = "UfeHex")] pub FieldElement);

impl fmt::LowerHex for HexFelt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let val = self.0;

        fmt::LowerHex::fmt(&val, f)
    }
}

impl From<FieldElement> for HexFelt {
    fn from(felt: FieldElement) -> Self {
        Self(felt)
    }
}

impl From<HexFelt> for FieldElement {
    fn from(hex_felt: HexFelt) -> Self {
        hex_felt.0
    }
}

impl From<Felt252Wrapper> for HexFelt {
    fn from(felt: Felt252Wrapper) -> HexFelt {
        HexFelt(felt.0)
    }
}

pub type ClassHash = HexFelt;
pub type ContractAddress = HexFelt;
pub type StorageKey = HexFelt;
pub type StorageValue = HexFelt;

#[derive(Deserialize, Serialize, Clone)]
pub struct GenesisData {
    pub contracts: Vec<(ContractAddress, ClassHash)>,
    pub sierra_class_hash_to_casm_class_hash: Vec<(ClassHash, ClassHash)>,
    pub storage: Vec<(ContractAddress, Vec<(StorageKey, StorageValue)>)>,
    pub strk_fee_token_address: ContractAddress,
    pub eth_fee_token_address: ContractAddress,
}

#[cfg(feature = "std")]
pub mod convert {
    use starknet_providers::sequencer::models::state_update::{StateDiff, StorageDiff};

    use super::*;

    impl From<StateDiff> for GenesisData {
        fn from(genesis_diff: StateDiff) -> Self {
            Self {
                contracts: convert_contract(&genesis_diff),
                sierra_class_hash_to_casm_class_hash: convert_sierra_class_hash(&genesis_diff),
                storage: convert_storage(&genesis_diff),
                strk_fee_token_address: *STRK_TOKEN_ADDR,
                eth_fee_token_address: *ETH_TOKEN_ADDR,
            }
        }
    }

    fn convert_contract(genesis_diff: &StateDiff) -> Vec<(ContractAddress, ClassHash)> {
        genesis_diff
            .deployed_contracts
            .iter()
            .map(|contract| (HexFelt(contract.address), HexFelt(contract.class_hash)))
            .collect()
    }

    fn convert_sierra_class_hash(genesis_diff: &StateDiff) -> Vec<(ClassHash, ClassHash)> {
        genesis_diff
            .declared_classes
            .iter()
            .map(|contract| (HexFelt(contract.class_hash), HexFelt(contract.compiled_class_hash)))
            .collect()
    }

    fn convert_storage(genesis_diff: &StateDiff) -> Vec<(ContractAddress, Vec<(StorageKey, StorageValue)>)> {
        genesis_diff
            .storage_diffs
            .iter()
            .map(|(contract_address, storage)| {
                (
                    HexFelt(*contract_address),
                    storage.iter().map(|StorageDiff { key, value }| (HexFelt(*key), HexFelt(*value))).collect(),
                )
            })
            .collect()
    }
}

#[derive(Constructor, Clone)]
pub struct GenesisLoader {
    base_path: PathBuf,
    data: GenesisData,
}

impl GenesisLoader {
    pub fn data(&self) -> &GenesisData {
        &self.data
    }
    pub fn base_path(&self) -> PathBuf {
        self.base_path.clone()
    }
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum ContractClass {
    Path { path: String, version: u8 },
    Class(StarknetContractClass),
}

/// A struct containing predeployed accounts info.
#[derive(Serialize, Deserialize)]
pub struct PredeployedAccount {
    pub contract_address: ContractAddress,
    pub class_hash: ClassHash,
    pub name: String,
    #[serde(serialize_with = "buffer_to_hex")]
    pub private_key: Option<Vec<u8>>,
    pub public_key: HexFelt,
}

pub fn buffer_to_hex<S>(buffer: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(inner_buffer) = buffer {
        let hex_string = format!("0x{}", hex::encode(inner_buffer));
        serializer.serialize_str(&hex_string)
    } else {
        serializer.serialize_none()
    }
}

pub fn hex_to_buffer<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let hex_string = String::deserialize(deserializer)?;
    if hex_string.is_empty() {
        Ok(None)
    } else {
        hex::decode(&hex_string).map(Some).map_err(|err| Error::custom(err.to_string()))
    }
}
