use std::num::NonZeroU128;

use blockifier::block::GasPrices;
use mc_db::DeoxysBackend;
use mc_rpc::deoxys_backend_client::get_block_by_block_hash;
use mc_sync::metrics::block_metrics::BlockMetrics;
use mp_digest_log::{find_starknet_block, FindLogError};
use mp_felt::Felt252Wrapper;
use mp_hashers::HasherT;
use mp_transactions::compute_hash::ComputeTransactionHash;
use mp_types::block::{DBlockT, DHashT, DHeaderT};
use num_traits::FromPrimitive;
use pallet_starknet_runtime_api::StarknetRuntimeApi;
use prometheus_endpoint::prometheus::core::Number;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use sc_client_api::backend::{Backend, StorageProvider};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::{Backend as _, HeaderBackend};
use sp_runtime::traits::Header as HeaderT;

fn sync_block<C, BE, H>(client: &C, header: &DHeaderT, block_metrics: Option<&BlockMetrics>) -> anyhow::Result<()>
where
    // TODO: refactor this!
    C: HeaderBackend<DBlockT> + StorageProvider<DBlockT, BE>,
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
    BE: Backend<DBlockT>,
    H: HasherT,
{
    // Before storing the new block in the Deoxys backend database, we want to make sure that the
    // wrapped Starknet block it contains is the same that we can find in the storage at this height.
    // Then we will store the two block hashes (wrapper and wrapped) alongside in our db.

    let substrate_block_hash = header.hash();
    match mp_digest_log::find_starknet_block(header.digest()) {
        Ok(digest_starknet_block) => {
            // Read the runtime storage in order to find the Starknet block stored under this Substrate block
            let opt_storage_starknet_block = get_block_by_block_hash(client, substrate_block_hash);
            match opt_storage_starknet_block {
                Ok(storage_starknet_block) => {
                    let (digest_starknet_block_hash, storage_starknet_block_hash) = rayon::join(
                        || digest_starknet_block.header().hash::<H>(),
                        || storage_starknet_block.header().hash::<H>(),
                    );
                    // Ensure the two blocks sources (chain storage and block digest) agree on the block content
                    if digest_starknet_block_hash != storage_starknet_block_hash {
                        Err(anyhow::anyhow!(
                            "Starknet block hash mismatch: deoxys consensus digest ({digest_starknet_block_hash:?}), \
                             db state ({storage_starknet_block_hash:?})"
                        ))
                    } else {
                        let chain_id = client.runtime_api().chain_id(substrate_block_hash)?;
                        let tx_hashes = digest_starknet_block
                            .transactions()
                            .par_iter()
                            .map(|tx| {
                                Felt252Wrapper::from(tx.compute_hash::<H>(
                                    chain_id,
                                    false,
                                    Some(digest_starknet_block.header().block_number),
                                ))
                                .into()
                            })
                            .collect();

                        // Success, we write the Starknet to Substate hashes mapping to db
                        let mapping_commitment = mc_db::MappingCommitment {
                            block_number: digest_starknet_block.header().block_number,
                            block_hash: substrate_block_hash,
                            starknet_block_hash: digest_starknet_block_hash.into(),
                            starknet_transaction_hashes: tx_hashes,
                        };

                        if let Some(block_metrics) = block_metrics {
                            let starknet_block = &digest_starknet_block.clone();
                            block_metrics.l2_block_number.set(starknet_block.header().block_number.into_f64());
                            let l1_gas_price = starknet_block.header().l1_gas_price.clone().unwrap_or(GasPrices {
                                eth_l1_gas_price: NonZeroU128::new(1).unwrap(),
                                strk_l1_gas_price: NonZeroU128::new(1).unwrap(),
                                eth_l1_data_gas_price: NonZeroU128::new(1).unwrap(),
                                strk_l1_data_gas_price: NonZeroU128::new(1).unwrap(),
                            });

                            // sending f64::MIN in case we exceed f64 (highly unlikely). The min numbers will
                            // allow dashboards to catch anomalies so that it can be investigated.
                            block_metrics
                                .transaction_count
                                .set(f64::from_u128(starknet_block.header().transaction_count).unwrap_or(f64::MIN));
                            block_metrics
                                .event_count
                                .set(f64::from_u128(starknet_block.header().event_count).unwrap_or(f64::MIN));
                            block_metrics
                                .l1_gas_price_wei
                                .set(f64::from_u128(l1_gas_price.eth_l1_gas_price.into()).unwrap_or(f64::MIN));
                            block_metrics
                                .l1_gas_price_strk
                                .set(f64::from_u128(l1_gas_price.strk_l1_gas_price.into()).unwrap_or(f64::MIN))
                        }

                        DeoxysBackend::mapping().write_hashes(mapping_commitment).map_err(|e| anyhow::anyhow!(e))
                    }
                }
                // If there is not Starknet block in this Substrate block, we write it in the db
                Err(_) => DeoxysBackend::mapping().write_none(substrate_block_hash).map_err(|e| anyhow::anyhow!(e)),
            }
        }
        // If there is not Starknet block in this Substrate block, we write it in the db
        Err(FindLogError::NotLog) => {
            DeoxysBackend::mapping().write_none(substrate_block_hash).map_err(|e| anyhow::anyhow!(e))
        }
        Err(FindLogError::MultipleLogs) => Err(anyhow::anyhow!("Multiple logs found")),
    }
}

fn sync_genesis_block<C, H>(_client: &C, header: &DHeaderT) -> anyhow::Result<()>
where
    C: HeaderBackend<DBlockT>,
    H: HasherT,
{
    let substrate_block_hash = header.hash();

    let block = match find_starknet_block(header.digest()) {
        Ok(block) => block,
        Err(FindLogError::NotLog) => {
            return DeoxysBackend::mapping().write_none(substrate_block_hash).map_err(|e| anyhow::anyhow!(e));
        }
        Err(FindLogError::MultipleLogs) => return Err(anyhow::anyhow!("Multiple logs found")),
    };
    let block_hash = block.header().hash::<H>();
    let mapping_commitment = mc_db::MappingCommitment::<DBlockT> {
        block_number: block.header().block_number,
        block_hash: substrate_block_hash,
        starknet_block_hash: block_hash.into(),
        starknet_transaction_hashes: Vec::new(),
    };

    DeoxysBackend::mapping().write_hashes(mapping_commitment)?;

    Ok(())
}

fn sync_one_block<C, BE, H>(
    client: &C,
    substrate_backend: &BE,
    sync_from: <DHeaderT as HeaderT>::Number,
    block_metrics: Option<&BlockMetrics>,
) -> anyhow::Result<bool>
where
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
    C: HeaderBackend<DBlockT> + StorageProvider<DBlockT, BE>,
    BE: Backend<DBlockT>,
    H: HasherT,
{
    // Fetch the leaves (latest unfinalized blocks) from the blockchain backend
    let mut leaves = substrate_backend.blockchain().leaves()?;
    if leaves.is_empty() {
        return Ok(false);
    }

    let mut operating_header = None;
    while let Some(checking_tip) = leaves.pop() {
        if let Some(checking_header) = fetch_header(substrate_backend.blockchain(), checking_tip, sync_from)? {
            operating_header = Some(checking_header);
            break;
        }
    }
    let operating_header = match operating_header {
        Some(operating_header) => operating_header,
        None => {
            return Ok(false);
        }
    };

    if *operating_header.number() == 0 {
        sync_genesis_block::<_, H>(client, &operating_header)?;
        Ok(true)
    } else {
        sync_block::<_, _, H>(client, &operating_header, block_metrics)?;
        Ok(true)
    }
}

pub fn sync_blocks<C, BE, H>(
    client: &C,
    substrate_backend: &BE,
    limit: usize,
    sync_from: <DHeaderT as HeaderT>::Number,
    block_metrics: Option<&BlockMetrics>,
) -> anyhow::Result<bool>
where
    C: ProvideRuntimeApi<DBlockT>,
    C::Api: StarknetRuntimeApi<DBlockT>,
    C: HeaderBackend<DBlockT> + StorageProvider<DBlockT, BE>,
    BE: Backend<DBlockT>,
    H: HasherT,
{
    let mut synced_any = false;

    for _ in 0..limit {
        synced_any = synced_any || sync_one_block::<_, _, H>(client, substrate_backend, sync_from, block_metrics)?;
    }

    Ok(synced_any)
}

fn fetch_header<BE>(
    substrate_backend: &BE,
    checking_tip: DHashT,
    sync_from: <DHeaderT as HeaderT>::Number,
) -> anyhow::Result<Option<DHeaderT>>
where
    BE: HeaderBackend<DBlockT>,
{
    if DeoxysBackend::mapping().is_synced(&checking_tip)? {
        return Ok(None);
    }

    match substrate_backend.header(checking_tip) {
        Ok(Some(checking_header)) if checking_header.number() >= &sync_from => Ok(Some(checking_header)),
        Ok(Some(_)) => Ok(None),
        Ok(None) | Err(_) => Err(anyhow::anyhow!("Header not found")),
    }
}
