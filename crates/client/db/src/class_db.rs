use dp_class::{ClassInfo, CompiledClass};
use rayon::{iter::ParallelIterator, slice::ParallelSlice};
use rocksdb::WriteOptions;
use starknet_core::types::Felt;

use crate::{
    db_block_id::{DbBlockId, DbBlockIdResolvable},
    storage_handler::DeoxysStorageError,
    Column, DatabaseExt, DeoxysBackend, WriteBatchWithTransaction, DB_UPDATES_BATCH_SIZE,
};

const LAST_KEY: &[u8] = &[0xFF; 64];

impl DeoxysBackend {
    fn class_db_get_encoded_kv<V: serde::de::DeserializeOwned>(
        &self,
        id: &DbBlockId,
        key: &Felt,
        pending_col: Column,
        nonpending_col: Column,
    ) -> Result<Option<(V, Option<u64>)>, DeoxysStorageError> {
        // todo: smallint here to avoid alloc
        let key_encoded = bincode::serialize(key)?;

        // Get from pending db, then normal db if not found.
        let block_n = match id {
            DbBlockId::Pending => {
                let col = self.db.get_column(pending_col);
                if let Some(res) = self.db.get_pinned_cf(&col, &key_encoded)? {
                    return Ok(Some(bincode::deserialize(&res)?)); // found in pending
                }

                None
            }
            DbBlockId::BlockN(block_n) => Some(*block_n),
        };

        let col = self.db.get_column(nonpending_col);
        let Some(val) = self.db.get_pinned_cf(&col, &key_encoded)? else { return Ok(None) };
        let val = bincode::deserialize(&val)?;

        Ok(Some((val, block_n)))
    }

    pub fn get_class_info(
        &self,
        id: &impl DbBlockIdResolvable,
        class_hash: &Felt,
    ) -> Result<Option<ClassInfo>, DeoxysStorageError> {
        let Some(id) = id.resolve_db_block_id(self)? else { return Ok(None) };

        let Some((info, block_n)) =
            self.class_db_get_encoded_kv::<ClassInfo>(&id, class_hash, Column::PendingClassInfo, Column::ClassInfo)?
        else {
            return Ok(None);
        };

        let valid = match (block_n, info.block_number) {
            (None, _) => true,
            (Some(block_n), Some(real_block_n)) => real_block_n >= block_n,
            _ => false,
        };
        if !valid {
            return Ok(None);
        }

        Ok(Some(info))
    }

    pub fn contains_class(&self, id: &impl DbBlockIdResolvable, class_hash: &Felt) -> Result<bool, DeoxysStorageError> {
        // TODO(perf): make fast path, this only needs one db contains() call and no deserialization in most cases (block id pending/latest)
        Ok(self.get_class_info(id, class_hash)?.is_some())
    }

    pub fn get_class(
        &self,
        id: &impl DbBlockIdResolvable,
        class_hash: &Felt,
    ) -> Result<Option<(ClassInfo, CompiledClass)>, DeoxysStorageError> {
        let Some(id) = id.resolve_db_block_id(self)? else { return Ok(None) };
        let Some(info) = self.get_class_info(&id, class_hash)? else { return Ok(None) };

        let (compiled_class, _block_n) =
            self.class_db_get_encoded_kv(&id, class_hash, Column::PendingClassCompiled, Column::ClassCompiled)?.ok_or(
                DeoxysStorageError::StorageDecodeError(crate::storage_handler::StorageType::CompiledContractClass),
            )?;

        Ok(Some((info, compiled_class)))
    }

    /// NB: This functions needs toruns on the rayon thread pool
    pub(crate) fn store_classes(
        &self,
        class_infos: &[(Felt, ClassInfo)],
        class_compiled: &[(Felt, CompiledClass)],
        col_info: Column,
        col_compiled: Column,
    ) -> Result<(), DeoxysStorageError> {
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);

        class_infos.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(col_info),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for (key, value) in chunk {
                    let key_bin = bincode::serialize(key)?;
                    // TODO: find a way to avoid this allocation
                    batch.put_cf(col, &key_bin, bincode::serialize(&value)?);
                }
                self.db.write_opt(batch, &writeopts)?;
                Ok::<_, DeoxysStorageError>(())
            },
        )?;

        class_compiled.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(col_compiled),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for (key, value) in chunk {
                    let key_bin = bincode::serialize(key)?;
                    // TODO: find a way to avoid this allocation
                    batch.put_cf(col, &key_bin, bincode::serialize(&value)?);
                }
                self.db.write_opt(batch, &writeopts)?;
                Ok::<_, DeoxysStorageError>(())
            },
        )?;

        Ok(())
    }

    /// NB: This functions needs toruns on the rayon thread pool
    pub(crate) fn class_db_store_block(
        &self,
        _block_number: u64,
        class_infos: &[(Felt, ClassInfo)],
        class_compiled: &[(Felt, CompiledClass)],
    ) -> Result<(), DeoxysStorageError> {
        self.store_classes(class_infos, class_compiled, Column::ClassInfo, Column::ClassCompiled)
    }

    /// NB: This functions needs toruns on the rayon thread pool
    pub(crate) fn class_db_store_pending(
        &self,
        class_infos: &[(Felt, ClassInfo)],
        class_compiled: &[(Felt, CompiledClass)],
    ) -> Result<(), DeoxysStorageError> {
        self.store_classes(class_infos, class_compiled, Column::PendingClassInfo, Column::PendingClassCompiled)
    }

    pub(crate) fn class_db_clear_pending(&self) -> Result<(), DeoxysStorageError> {
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);

        self.db.delete_range_cf_opt(&self.db.get_column(Column::PendingClassInfo), &[] as _, LAST_KEY, &writeopts)?;
        self.db.delete_range_cf_opt(
            &self.db.get_column(Column::PendingClassCompiled),
            &[] as _,
            LAST_KEY,
            &writeopts,
        )?;

        Ok(())
    }
}
