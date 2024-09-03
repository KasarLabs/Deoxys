use std::collections::HashSet;

use mp_class::{ClassInfo, CompiledClass};
use rayon::{iter::ParallelIterator, slice::ParallelSlice};
use rocksdb::WriteOptions;
use starknet_types_core::felt::Felt;

use crate::{
    db_block_id::{DbBlockId, DbBlockIdResolvable},
    Column, DatabaseExt, MadaraBackend, MadaraStorageError, WriteBatchWithTransaction, DB_UPDATES_BATCH_SIZE,
};

const LAST_KEY: &[u8] = &[0xFF; 64];

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ClassInfoWithBlockNumber {
    class_info: ClassInfo,
    block_id: DbBlockId,
}

impl MadaraBackend {
    fn class_db_get_encoded_kv<V: serde::de::DeserializeOwned>(
        &self,
        is_pending: bool,
        key: &Felt,
        pending_col: Column,
        nonpending_col: Column,
    ) -> Result<Option<V>, MadaraStorageError> {
        // todo: smallint here to avoid alloc
        log::debug!("get encoded {key:#x}");
        let key_encoded = bincode::serialize(key)?;

        // Get from pending db, then normal db if not found.
        if is_pending {
            let col = self.db.get_column(pending_col);
            if let Some(res) = self.db.get_pinned_cf(&col, &key_encoded)? {
                return Ok(Some(bincode::deserialize(&res)?)); // found in pending
            }
        }
        log::debug!("get encoded: not in pending");

        let col = self.db.get_column(nonpending_col);
        let Some(val) = self.db.get_pinned_cf(&col, &key_encoded)? else { return Ok(None) };
        let val = bincode::deserialize(&val)?;

        Ok(Some(val))
    }

    pub fn get_class_info(
        &self,
        id: &impl DbBlockIdResolvable,
        class_hash: &Felt,
    ) -> Result<Option<ClassInfo>, MadaraStorageError> {
        let Some(requested_id) = id.resolve_db_block_id(self)? else { return Ok(None) };

        log::debug!("class info {requested_id:?} {class_hash:#x}");

        let Some(info) = self.class_db_get_encoded_kv::<ClassInfoWithBlockNumber>(
            requested_id.is_pending(),
            class_hash,
            Column::PendingClassInfo,
            Column::ClassInfo,
        )?
        else {
            return Ok(None);
        };

        log::debug!("class info got {:?}", info.block_id);

        let valid = match (requested_id, info.block_id) {
            (DbBlockId::Pending, _) => true,
            (DbBlockId::BlockN(block_n), DbBlockId::BlockN(real_block_n)) => real_block_n <= block_n,
            _ => false,
        };
        if !valid {
            return Ok(None);
        }
        log::debug!("valid");

        Ok(Some(info.class_info))
    }

    pub fn contains_class(&self, id: &impl DbBlockIdResolvable, class_hash: &Felt) -> Result<bool, MadaraStorageError> {
        // TODO(perf): make fast path, this only needs one db contains() call and no deserialization in most cases (block id pending/latest)
        Ok(self.get_class_info(id, class_hash)?.is_some())
    }

    pub fn get_class(
        &self,
        id: &impl DbBlockIdResolvable,
        class_hash: &Felt,
    ) -> Result<Option<(ClassInfo, CompiledClass)>, MadaraStorageError> {
        let Some(id) = id.resolve_db_block_id(self)? else { return Ok(None) };
        let Some(info) = self.get_class_info(&id, class_hash)? else { return Ok(None) };

        log::debug!("get_class {:?} {:#x}", id, class_hash);
        let compiled_class = self
            .class_db_get_encoded_kv::<CompiledClass>(
                id.is_pending(),
                class_hash,
                Column::PendingClassCompiled,
                Column::ClassCompiled,
            )?
            .ok_or(MadaraStorageError::InconsistentStorage("Class compiled not found while class info is".into()))?;

        Ok(Some((info, compiled_class)))
    }

    /// NB: This functions needs to run on the rayon thread pool
    pub(crate) fn store_classes(
        &self,
        block_id: DbBlockId,
        class_infos: &[(Felt, ClassInfo)],
        class_compiled: &[(Felt, CompiledClass)],
        col_info: Column,
        col_compiled: Column,
    ) -> Result<(), MadaraStorageError> {
        let mut writeopts = WriteOptions::new();
        writeopts.disable_wal(true);

        // Check if the class is already in the db, if so, skip it
        // This check is needed because blocks are fetched and converted in parallel
        // TODO(merge): this should be removed after block import refactor
        let ignore_class: HashSet<_> = if let DbBlockId::BlockN(block_n) = block_id {
            class_infos
                .iter()
                .filter_map(|(key, _)| match self.get_class_info(&DbBlockId::BlockN(block_n), key) {
                    Ok(Some(_)) => Some(*key),
                    _ => None,
                })
                .collect()
        } else {
            HashSet::new()
        };

        class_infos.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(col_info),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for (key, value) in chunk {
                    if ignore_class.contains(key) {
                        continue;
                    }
                    let key_bin = bincode::serialize(key)?;
                    // TODO: find a way to avoid this allocation
                    batch.put_cf(
                        col,
                        &key_bin,
                        bincode::serialize(&ClassInfoWithBlockNumber { class_info: value.clone(), block_id })?,
                    );
                }
                self.db.write_opt(batch, &writeopts)?;
                Ok::<_, MadaraStorageError>(())
            },
        )?;

        class_compiled.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(col_compiled),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for (key, value) in chunk {
                    if ignore_class.contains(key) {
                        continue;
                    }
                    let key_bin = bincode::serialize(key)?;
                    // TODO: find a way to avoid this allocation
                    batch.put_cf(col, &key_bin, bincode::serialize(&value)?);
                }
                self.db.write_opt(batch, &writeopts)?;
                Ok::<_, MadaraStorageError>(())
            },
        )?;

        Ok(())
    }

    /// NB: This functions needs to run on the rayon thread pool
    pub(crate) fn class_db_store_block(
        &self,
        block_number: u64,
        class_infos: &[(Felt, ClassInfo)],
        class_compiled: &[(Felt, CompiledClass)],
    ) -> Result<(), MadaraStorageError> {
        self.store_classes(
            DbBlockId::BlockN(block_number),
            class_infos,
            class_compiled,
            Column::ClassInfo,
            Column::ClassCompiled,
        )
    }

    /// NB: This functions needs to run on the rayon thread pool
    pub(crate) fn class_db_store_pending(
        &self,
        class_infos: &[(Felt, ClassInfo)],
        class_compiled: &[(Felt, CompiledClass)],
    ) -> Result<(), MadaraStorageError> {
        self.store_classes(
            DbBlockId::Pending,
            class_infos,
            class_compiled,
            Column::PendingClassInfo,
            Column::PendingClassCompiled,
        )
    }

    pub(crate) fn class_db_clear_pending(&self) -> Result<(), MadaraStorageError> {
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
