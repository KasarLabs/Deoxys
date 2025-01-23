// mod config;
mod constants;
mod error;
mod events;
mod filter;
mod storage;

// pub use config::BloomConfig;
pub use constants::*;
pub use error::BloomError;
pub use filter::{BloomFilter, PreCalculatedHashes};
pub use storage::{AtomicBitStore, BitStore};

pub use events::{EventBloomReader, EventBloomSearcher, EventBloomWriter};

#[cfg(test)]
mod tests;
