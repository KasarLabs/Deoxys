use crate::state_update::StateUpdate;
use assert_matches::assert_matches;
use starknet_accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet_core::types::contract::SierraClass;
use starknet_core::types::{BlockId, BlockTag, Call, TransactionReceipt, TransactionReceiptWithBlockInfo};
use starknet_core::utils::get_selector_from_name;
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider, ProviderError};
use starknet_signers::{LocalWallet, SigningKey};
use starknet_types_core::felt::Felt;
use std::future::Future;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use m_cairo_test_contracts::{APPCHAIN_CONTRACT_SIERRA, MESSAGING_CONTRACT_SIERRA};
use std::time::Duration;
use url::Url;

pub const DEPLOYER_ADDRESS: &str = "0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d";
pub const DEPLOYER_PRIVATE_KEY: &str = "0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07";
pub const UDC_ADDRESS: &str = "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf";
pub const MADARA_PORT: &str = "19944";
pub const MADARA_BINARY_PATH: &str = "../../../../test-artifacts/madara";
pub const MADARA_CONFIG_PATH: &str = "../../../../configs/presets/devnet.yaml";

// starkli class-hash crates/client/settlement_client/src/starknet/test_contracts/appchain_test.casm.json
pub const APPCHAIN_CONTRACT_CASM_HASH: &str = "0x07f36e830605ddeb7c4c094639b628de297cbf61f45385b1fc3231029922b30b";
// starkli class-hash crates/client/settlement_client/src/starknet/test_contracts/messaging_test.casm.json
pub const MESSAGING_CONTRACT_CASM_HASH: &str = "0x077de37b708f9abe01c1a797856398c5e1e5dfde8213f884668fa37b13d77e30";

pub type StarknetAccount = SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>;
pub type TransactionReceiptResult = Result<TransactionReceiptWithBlockInfo, ProviderError>;

pub struct MadaraProcess {
    pub process: Child,
    #[allow(dead_code)]
    pub binary_path: PathBuf,
}

impl MadaraProcess {
    pub fn new(binary_path: PathBuf) -> Result<Self, std::io::Error> {
        let process = Command::new(&binary_path)
            .arg("--name")
            .arg("madara")
            .arg("--base-path")
            .arg("../madara-db33")
            .arg("--rpc-port")
            .arg(MADARA_PORT)
            .arg("--rpc-cors")
            .arg("*")
            .arg("--rpc-external")
            .arg("--devnet")
            .arg("--chain-config-path")
            .arg(MADARA_CONFIG_PATH)
            .arg("--feeder-gateway-enable")
            .arg("--gateway-enable")
            .arg("--gateway-external")
            .arg("--gateway-port")
            .arg("8080")
            .arg("--no-l1-sync")
            .arg("--chain-config-override=block_time=5s,pending_block_update_time=1s")
            .spawn()?;

        wait_for_port(MADARA_PORT.parse().unwrap(), 2, 10);

        Ok(Self { process, binary_path })
    }
}

impl Drop for MadaraProcess {
    fn drop(&mut self) {
        if let Err(e) = self.process.kill() {
            eprintln!("Failed to kill Madara process: {}", e);
        } else {
            Command::new("rm").arg("-rf").arg("../madara-db33").status().expect("Failed to delete the madara db");
            println!("Madara process killed successfully");
        }
    }
}

pub async fn prepare_starknet_client_test() -> anyhow::Result<(StarknetAccount, Felt, MadaraProcess)> {
    let madara = MadaraProcess::new(PathBuf::from(MADARA_BINARY_PATH))?;
    let account = starknet_account()?;
    let deployed_appchain_contract_address =
        deploy_contract(&account, APPCHAIN_CONTRACT_SIERRA, APPCHAIN_CONTRACT_CASM_HASH).await?;
    Ok((account, deployed_appchain_contract_address, madara))
}

pub async fn prepare_starknet_client_messaging_test() -> anyhow::Result<(StarknetAccount, Felt, MadaraProcess)> {
    let madara = MadaraProcess::new(PathBuf::from(MADARA_BINARY_PATH))?;
    let account = starknet_account()?;
    let deployed_appchain_contract_address =
        deploy_contract(&account, MESSAGING_CONTRACT_SIERRA, MESSAGING_CONTRACT_CASM_HASH).await?;
    Ok((account, deployed_appchain_contract_address, madara))
}

pub async fn send_state_update(
    account: &StarknetAccount,
    appchain_contract_address: Felt,
    update: StateUpdate,
) -> anyhow::Result<u64> {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("update_state")?,
            calldata: vec![Felt::from(update.block_number), update.global_root, update.block_hash],
        }])
        .send()
        .await?;
    let receipt = get_transaction_receipt(account.provider(), call.transaction_hash).await?;

    let latest_block_number_recorded = account.provider().block_number().await?;

    match receipt.block.block_number() {
        Some(block_number) => Ok(block_number),
        None => Ok(latest_block_number_recorded + 1),
    }
}

pub async fn fire_messaging_event(account: &StarknetAccount, appchain_contract_address: Felt) -> anyhow::Result<u64> {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("fire_event")?,
            calldata: vec![],
        }])
        .send()
        .await?;
    let receipt = get_transaction_receipt(account.provider(), call.transaction_hash).await?;

    let latest_block_number_recorded = account.provider().block_number().await?;

    match receipt.block.block_number() {
        Some(block_number) => Ok(block_number),
        None => Ok(latest_block_number_recorded + 1),
    }
}

pub async fn cancel_messaging_event(account: &StarknetAccount, appchain_contract_address: Felt) -> anyhow::Result<u64> {
    let call = account
        .execute_v1(vec![Call {
            to: appchain_contract_address,
            selector: get_selector_from_name("set_is_canceled")?,
            calldata: vec![Felt::ONE],
        }])
        .send()
        .await?;
    let receipt = get_transaction_receipt(account.provider(), call.transaction_hash).await?;

    let latest_block_number_recorded = account.provider().block_number().await?;

    match receipt.block.block_number() {
        Some(block_number) => Ok(block_number),
        None => Ok(latest_block_number_recorded + 1),
    }
}

pub fn starknet_account() -> anyhow::Result<StarknetAccount> {
    let provider =
        JsonRpcClient::new(HttpTransport::new(Url::parse(format!("http://127.0.0.1:{}", MADARA_PORT).as_str())?));
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(Felt::from_str(DEPLOYER_PRIVATE_KEY)?));
    let mut account = SingleOwnerAccount::new(
        provider,
        signer,
        Felt::from_str(DEPLOYER_ADDRESS)?,
        // MADARA_DEVNET
        Felt::from_str("0x4D41444152415F4445564E4554")?,
        ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::Pending));
    Ok(account)
}

pub async fn deploy_contract(account: &StarknetAccount, sierra: &[u8], casm_hash: &str) -> anyhow::Result<Felt> {
    let contract_artifact: SierraClass = serde_json::from_slice(sierra)?;
    let flattened_class = contract_artifact.flatten()?;
    let result = account.declare_v2(Arc::new(flattened_class), Felt::from_str(casm_hash)?).send().await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let deployment = account
        .execute_v3(vec![Call {
            to: Felt::from_str(UDC_ADDRESS)?,
            selector: get_selector_from_name("deployContract")?,
            calldata: vec![result.class_hash, Felt::ZERO, Felt::ZERO, Felt::ZERO],
        }])
        .send()
        .await?;
    let deployed_contract_address =
        get_deployed_contract_address(deployment.transaction_hash, account.provider()).await?;
    tokio::time::sleep(Duration::from_secs(5)).await;
    Ok(deployed_contract_address)
}

pub async fn get_deployed_contract_address(
    txn_hash: Felt,
    provider: &JsonRpcClient<HttpTransport>,
) -> anyhow::Result<Felt> {
    let deploy_tx_receipt = get_transaction_receipt(provider, txn_hash).await?;
    let contract_address = assert_matches!(
        deploy_tx_receipt,
        TransactionReceiptWithBlockInfo { receipt: TransactionReceipt::Invoke(receipt), .. } => {
            receipt.events.iter().find(|e| e.keys[0] == get_selector_from_name("ContractDeployed").unwrap()).unwrap().data[0]
        }
    );
    Ok(contract_address)
}

pub async fn get_transaction_receipt(
    rpc: &JsonRpcClient<HttpTransport>,
    transaction_hash: Felt,
) -> TransactionReceiptResult {
    // there is a delay between the transaction being available at the client
    // and the pending tick of the block, hence sleeping for 500ms
    assert_poll(|| async { rpc.get_transaction_receipt(transaction_hash).await.is_ok() }, 500, 20).await;
    rpc.get_transaction_receipt(transaction_hash).await
}

pub async fn assert_poll<F, Fut>(f: F, polling_time_ms: u64, max_poll_count: u32)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    for _poll_count in 0..max_poll_count {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(polling_time_ms)).await;
    }
    panic!("Max poll count exceeded.");
}

fn wait_for_port(port: u16, timeout_secs: u64, max_retries: u32) -> bool {
    let mut attempts = 0;
    println!("Waiting for port {} to be available...", port);

    while attempts < max_retries {
        if check_port(port, timeout_secs) {
            println!("Port {} is now available! (attempt {}/{})", port, attempts + 1, max_retries);
            return true;
        }

        attempts += 1;
        if attempts < max_retries {
            println!("Port {} not available, retrying... (attempt {}/{})", port, attempts, max_retries);
            thread::sleep(Duration::from_secs(timeout_secs));
        }
    }

    println!("Port {} not available after {} attempts", port, max_retries);
    false
}

fn check_port(port: u16, timeout_secs: u64) -> bool {
    TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(timeout_secs))
        .is_ok()
}
