use crate::client::{ClientTrait, ClientType};
use alloy::primitives::B256;
use futures::{Stream, StreamExt};
use mc_db::l1_db::LastSyncedEventBlock;
use mc_db::MadaraBackend;
use mc_mempool::{Mempool, MempoolProvider};
use mp_utils::service::ServiceContext;
use starknet_api::core::{ChainId, ContractAddress, EntryPointSelector, Nonce};
use starknet_api::transaction::{Calldata, L1HandlerTransaction, TransactionVersion};
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use tracing::{error, info};

#[derive(Clone, Debug)]
pub struct CommonMessagingEventData {
    pub from: Felt,
    pub to: Felt,
    pub selector: Felt,
    pub nonce: Felt,
    pub payload: Vec<Felt>,
    pub fee: Option<u128>,
    pub transaction_hash: Felt,
    pub message_hash: Option<Felt>,
    pub block_number: u64,
    pub event_index: Option<u64>,
}

pub async fn sync<C, S>(
    settlement_client: Arc<Box<dyn ClientTrait<Config = C, StreamType = S>>>,
    backend: Arc<MadaraBackend>,
    chain_id: ChainId,
    mempool: Arc<Mempool>,
    mut ctx: ServiceContext,
) -> anyhow::Result<()>
where
    S: Stream<Item = Option<anyhow::Result<CommonMessagingEventData>>> + Send + 'static,
{
    info!("⟠ Starting L1 Messages Syncing...");

    let last_synced_event_block = match backend.messaging_last_synced_l1_block_with_event() {
        Ok(Some(blk)) => blk,
        Ok(None) => {
            unreachable!("Should never be None")
        }
        Err(e) => {
            error!("⟠ Madara Messaging DB unavailable: {:?}", e);
            return Err(e.into());
        }
    };

    let stream = settlement_client.get_messaging_stream(last_synced_event_block).await?;
    let mut event_stream = Box::pin(stream);

    while let Some(Some(event_result)) = ctx.run_until_cancelled(event_stream.next()).await {
        if let Some(event) = event_result {
            let event_data = event?;
            let tx = parse_handle_message_transaction(&event_data)?;
            let tx_nonce = tx.nonce;

            // Skip if already processed
            if backend.has_l1_messaging_nonce(tx_nonce)? {
                info!("Event already processed");
                return Ok(());
            }

            info!(
                "Processing Message from block: {:?}, transaction_hash: {:?}, fromAddress: {:?}",
                event_data.block_number,
                format!("0x{}", event_data.transaction_hash.to_hex_string()),
                format!("0x{}", event_data.transaction_hash.to_hex_string()),
            );

            // Check message hash and cancellation
            let event_hash = settlement_client.get_messaging_hash(&event_data)?;
            let converted_event_hash = match settlement_client.get_client_type() {
                ClientType::ETH => B256::from_slice(event_hash.as_slice()).to_string(),
                ClientType::STARKNET => Felt::from_bytes_be_slice(event_hash.as_slice()).to_hex_string(),
            };
            info!("Checking for cancellation, event hash: {:?}", converted_event_hash);

            let cancellation_timestamp = settlement_client.get_l1_to_l2_message_cancellations(event_hash).await?;
            if cancellation_timestamp != Felt::ZERO {
                info!("Message was cancelled in block at timestamp: {:?}", cancellation_timestamp);
                handle_cancelled_message(backend, tx_nonce)?;
                return Ok(());
            }

            // Process message
            match process_message(&backend, &event_data, &chain_id, mempool.clone()).await {
                Ok(Some(tx_hash)) => {
                    info!(
                        "Message from block: {:?} submitted, transaction hash: {:?}",
                        event_data.block_number, tx_hash
                    );

                    let block_sent =
                        LastSyncedEventBlock::new(event_data.block_number, event_data.event_index.unwrap_or(0));
                    backend.messaging_update_last_synced_l1_block_with_event(block_sent)?;
                }
                Ok(None) => {}
                Err(e) => {
                    error!(
                        "Unexpected error while processing Message from block: {:?}, error: {:?}",
                        event_data.block_number, e
                    );
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

fn handle_cancelled_message(backend: Arc<MadaraBackend>, nonce: Nonce) -> anyhow::Result<()> {
    match backend.has_l1_messaging_nonce(nonce) {
        Ok(false) => {
            backend.set_l1_messaging_nonce(nonce)?;
        }
        Ok(true) => {}
        Err(e) => {
            error!("Unexpected DB error: {:?}", e);
            return Err(e.into());
        }
    }
    Ok(())
}

pub fn parse_handle_message_transaction(event: &CommonMessagingEventData) -> anyhow::Result<L1HandlerTransaction> {
    let calldata: Calldata = {
        let mut calldata: Vec<_> = Vec::with_capacity(event.payload.len() + 1);
        calldata.push(event.from);
        calldata.extend(event.payload.clone());
        Calldata(Arc::new(calldata))
    };

    Ok(L1HandlerTransaction {
        nonce: Nonce(event.nonce),
        contract_address: ContractAddress(event.to.try_into()?),
        entry_point_selector: EntryPointSelector(event.selector),
        calldata,
        version: TransactionVersion(Felt::ZERO),
    })
}

async fn process_message(
    backend: &MadaraBackend,
    event: &CommonMessagingEventData,
    _chain_id: &ChainId,
    mempool: Arc<Mempool>,
) -> anyhow::Result<Option<Felt>> {
    let transaction = parse_handle_message_transaction(event)?;
    let tx_nonce = transaction.nonce;
    let fees = event.fee;

    // Ensure that L1 message has not been executed
    match backend.has_l1_messaging_nonce(tx_nonce) {
        Ok(false) => {
            backend.set_l1_messaging_nonce(tx_nonce)?;
        }
        Ok(true) => {
            tracing::debug!("⟠ Event already processed: {:?}", transaction);
            return Ok(None);
        }
        Err(e) => {
            error!("⟠ Unexpected DB error: {:?}", e);
            return Err(e.into());
        }
    };

    let res = mempool.tx_accept_l1_handler(transaction.into(), fees.unwrap_or(0))?;

    Ok(Some(res.transaction_hash))
}

#[cfg(test)]
mod messaging_module_tests {
    use super::*;
    use crate::client::{DummyConfig, DummyStream, MockClientTrait};
    use futures::stream;
    use mc_db::DatabaseService;
    use mc_mempool::{GasPriceProvider, L1DataProvider, MempoolLimits};
    use mp_chain_config::ChainConfig;
    use rstest::{fixture, rstest};
    use starknet_types_core::felt::Felt;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::timeout;

    // Helper function to create a mock event
    fn create_mock_event(block_number: u64, nonce: u64) -> CommonMessagingEventData {
        CommonMessagingEventData {
            block_number,
            transaction_hash: Felt::from(1),
            event_index: Some(0),
            from: Felt::from(123),
            to: Felt::from(456),
            selector: Felt::from(789),
            payload: vec![Felt::from(1), Felt::from(2)],
            nonce: Felt::from(nonce),
            fee: Some(1000),
            message_hash: None,
        }
    }

    struct MessagingTestRunner {
        client: MockClientTrait,
        db: Arc<DatabaseService>,
        mempool: Arc<Mempool>,
        ctx: ServiceContext,
        chain_config: Arc<ChainConfig>,
    }

    #[fixture]
    async fn setup_messaging_tests() -> MessagingTestRunner {
        // Set up chain info
        let chain_config = Arc::new(ChainConfig::madara_test());

        // Set up database paths
        let temp_dir = TempDir::new().expect("issue while creating temporary directory");
        let base_path = temp_dir.path().join("data");
        let backup_dir = Some(temp_dir.path().join("backups"));

        // Initialize database service
        let db = Arc::new(
            DatabaseService::new(&base_path, backup_dir, false, chain_config.clone(), Default::default())
                .await
                .expect("Failed to create database service"),
        );

        let l1_gas_setter = GasPriceProvider::new();
        let l1_data_provider: Arc<dyn L1DataProvider> = Arc::new(l1_gas_setter.clone());

        let mempool = Arc::new(Mempool::new(
            Arc::clone(db.backend()),
            Arc::clone(&l1_data_provider),
            MempoolLimits::for_testing(),
        ));

        // Create a new context for mocking the static new() method
        let ctx = MockClientTrait::new_context();
        ctx.expect().returning(|_| Ok(MockClientTrait::default()));
        let mock_client = MockClientTrait::new(DummyConfig).await.expect("Unable to init new mock client");

        let ctx = ServiceContext::new_for_testing();

        MessagingTestRunner { client: mock_client, db, mempool, ctx, chain_config }
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_processes_new_message(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx, chain_config } = setup_messaging_tests.await;

        // Setup mock event
        let mock_event = create_mock_event(100, 1);
        let event_clone = mock_event.clone();

        let backend = db.backend();

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream
        client
            .expect_get_messaging_stream()
            .times(1)
            .returning(move |_| Ok(Box::pin(stream::iter(vec![Some(Ok(mock_event.clone()))]))));

        // Mock get_messaging_hash
        client.expect_get_messaging_hash().times(1).returning(|_| Ok(vec![0u8; 32]));

        // Mock get_client_type
        client.expect_get_client_type().times(1).returning(|| ClientType::ETH);

        // Mock get_l1_to_l2_message_cancellations
        client.expect_get_l1_to_l2_message_cancellations().times(1).returning(|_| Ok(Felt::ZERO));

        let client: Arc<Box<dyn ClientTrait<Config = DummyConfig, StreamType = DummyStream>>> =
            Arc::new(Box::new(client));

        timeout(
            Duration::from_secs(1),
            sync(client, backend.clone(), chain_config.chain_id.clone(), mempool.clone(), ctx),
        )
        .await??;

        // Verify the message was processed
        assert!(backend.has_l1_messaging_nonce(Nonce(event_clone.nonce))?);

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_handles_cancelled_message(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx, chain_config } = setup_messaging_tests.await;

        let backend = db.backend();

        // Setup mock event
        let mock_event = create_mock_event(100, 1);
        let event_clone = mock_event.clone();

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream
        client
            .expect_get_messaging_stream()
            .times(1)
            .returning(move |_| Ok(Box::pin(stream::iter(vec![Some(Ok(mock_event.clone()))]))));

        // Mock get_messaging_hash
        client.expect_get_messaging_hash().times(1).returning(|_| Ok(vec![0u8; 32]));

        // Mock get_client_type
        client.expect_get_client_type().times(1).returning(|| ClientType::ETH);

        // Mock get_l1_to_l2_message_cancellations - return non-zero to indicate cancellation
        client.expect_get_l1_to_l2_message_cancellations().times(1).returning(|_| Ok(Felt::from(12345)));

        let client: Arc<Box<dyn ClientTrait<Config = DummyConfig, StreamType = DummyStream>>> =
            Arc::new(Box::new(client));

        timeout(
            Duration::from_secs(1),
            sync(client, backend.clone(), chain_config.chain_id.clone(), mempool.clone(), ctx),
        )
        .await??;

        // Verify the cancelled message was handled correctly
        assert!(backend.has_l1_messaging_nonce(Nonce(event_clone.nonce))?);

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_skips_already_processed_message(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx, chain_config } = setup_messaging_tests.await;

        let backend = db.backend();

        // Setup mock event
        let mock_event = create_mock_event(100, 1);

        // Pre-set the nonce as processed
        backend.set_l1_messaging_nonce(Nonce(mock_event.nonce))?;

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream
        client
            .expect_get_messaging_stream()
            .times(1)
            .returning(move |_| Ok(Box::pin(stream::iter(vec![Some(Ok(mock_event.clone()))]))));

        // Mock get_messaging_hash - should not be called
        client.expect_get_messaging_hash().times(0);

        let client: Arc<Box<dyn ClientTrait<Config = DummyConfig, StreamType = DummyStream>>> =
            Arc::new(Box::new(client));

        timeout(
            Duration::from_secs(1),
            sync(client, backend.clone(), chain_config.chain_id.clone(), mempool.clone(), ctx),
        )
        .await??;

        Ok(())
    }

    #[rstest]
    #[tokio::test]
    async fn test_sync_handles_stream_errors(
        #[future] setup_messaging_tests: MessagingTestRunner,
    ) -> anyhow::Result<()> {
        let MessagingTestRunner { mut client, db, mempool, ctx, chain_config } = setup_messaging_tests.await;

        let backend = db.backend();

        // Setup mock for last synced block
        backend.messaging_update_last_synced_l1_block_with_event(LastSyncedEventBlock::new(99, 0))?;

        // Mock get_messaging_stream to return error
        client
            .expect_get_messaging_stream()
            .times(1)
            .returning(move |_| Ok(Box::pin(stream::iter(vec![Some(Err(anyhow::anyhow!("Stream error")))]))));

        let client: Arc<Box<dyn ClientTrait<Config = DummyConfig, StreamType = DummyStream>>> =
            Arc::new(Box::new(client));

        let result = sync(client, backend.clone(), chain_config.chain_id.clone(), mempool.clone(), ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Stream error"));

        Ok(())
    }
}
