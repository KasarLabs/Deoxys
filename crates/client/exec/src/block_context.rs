use blockifier::{
    blockifier::{config::TransactionExecutorConfig, transaction_executor::TransactionExecutor},
    context::{BlockContext, ChainInfo, FeeTokenAddresses},
    state::cached_state::CachedState,
};
use dc_db::{db_block_id::DbBlockId, DeoxysBackend};
use dp_block::{header::L1DataAvailabilityMode, DeoxysMaybePendingBlockInfo};
use starknet_api::block::{BlockNumber, BlockTimestamp};

use crate::{blockifier_state_adapter::BlockifierStateAdapter, Error};

pub struct ExecutionContext<'a> {
    pub(crate) backend: &'a DeoxysBackend,
    pub(crate) block_context: BlockContext,
    pub(crate) db_id: DbBlockId,
}

impl<'a> ExecutionContext<'a> {
    pub fn tx_executor(&self) -> TransactionExecutor<BlockifierStateAdapter<'a>> {
        TransactionExecutor::new(
            self.init_cached_state(),
            self.block_context.clone(),
            // No concurrency yet.
            TransactionExecutorConfig { concurrency_config: Default::default() },
        )
    }

    pub fn init_cached_state(&self) -> CachedState<BlockifierStateAdapter<'a>> {
        let on_top_of = match self.db_id {
            DbBlockId::Pending => Some(DbBlockId::Pending),
            DbBlockId::BlockN(block_n) => {
                // We exec on top of the previous block. None means we are executing genesis.
                block_n.checked_sub(1).map(DbBlockId::BlockN)
            }
        };

        CachedState::new(BlockifierStateAdapter::new(self.backend, on_top_of))
    }

    pub fn new(backend: &'a DeoxysBackend, block_info: &DeoxysMaybePendingBlockInfo) -> Result<Self, Error> {
        let (db_id, protocol_version, block_number, block_timestamp, sequencer_address, l1_gas_price, l1_da_mode) =
            match block_info {
                DeoxysMaybePendingBlockInfo::Pending(block) => (
                    DbBlockId::Pending,
                    block.header.protocol_version,
                    backend.get_latest_block_n()?.map(|el| el + 1).unwrap_or(0), // when the block is pending, we use the latest block n + 1
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price.clone(),
                    block.header.l1_da_mode,
                ),
                DeoxysMaybePendingBlockInfo::NotPending(block) => (
                    DbBlockId::BlockN(block.header.block_number),
                    block.header.protocol_version,
                    block.header.block_number,
                    block.header.block_timestamp,
                    block.header.sequencer_address,
                    block.header.l1_gas_price.clone(),
                    block.header.l1_da_mode,
                ),
            };

        let versioned_constants = backend.chain_config().exec_constants_by_protocol_version(protocol_version)?;
        let chain_info = ChainInfo {
            chain_id: backend.chain_config().chain_id.clone(),
            fee_token_addresses: FeeTokenAddresses {
                strk_fee_token_address: backend.chain_config().native_fee_token_address,
                eth_fee_token_address: backend.chain_config().parent_fee_token_address,
            },
        };
        let block_info = blockifier::blockifier::block::BlockInfo {
            block_number: BlockNumber(block_number),
            block_timestamp: BlockTimestamp(block_timestamp),
            sequencer_address: sequencer_address
                .try_into()
                .map_err(|_| Error::InvalidSequencerAddress(sequencer_address))?,
            gas_prices: (&l1_gas_price).into(),
            // TODO: Verify if this is correct
            use_kzg_da: l1_da_mode == L1DataAvailabilityMode::Blob,
        };

        Ok(ExecutionContext {
            block_context: BlockContext::new(
                block_info,
                chain_info,
                versioned_constants,
                backend.chain_config().bouncer_config.clone(),
            ),
            db_id,
            backend,
        })
    }
}
