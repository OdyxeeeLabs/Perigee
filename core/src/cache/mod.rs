//! Two-tier simulation cache: in-memory L1 + disk-persistent L2.
//!
//! The in-memory side (Moka) lives on [`crate::simulation::SimulationCache`]
//! for backward compatibility; this module only ships the L2 layer plus the
//! shared error type that spans both tiers. `SimulationCache` exposes
//! `with_disk_cache` to attach an L2 store — when it is set, reads walk
//! L1 → L2 → miss (and promote L2 hits into L1), and writes populate both
//! layers so state survives restarts.

pub mod disk;

pub use disk::{DiskCache, DiskCacheConfig};

use thiserror::Error;

/// Errors surfaced by the cache subsystem.
///
/// These bubble up from Sled's disk store, bincode (de)serialisation, and
/// I/O when opening a backing directory. The main service converts them
/// into HTTP 500 via the `AppError` layer; callers inside the cache path
/// normally treat L2 errors as misses and log-and-continue rather than
/// failing the whole simulation.
#[derive(Error, Debug)]
pub enum CacheError {
    #[error("disk cache I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("disk cache backend error: {0}")]
    Backend(#[from] sled::Error),

    #[error("cache payload (de)serialisation error: {0}")]
    Serialization(#[from] bincode::Error),
}
