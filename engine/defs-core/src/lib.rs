//! # DEFS — Data-Enriched File System v2
//!
//! An AI-native, content-aware filesystem where every file is a Particle,
//! every property is a Dimension, every value is a Wavelet,
//! and directories are replaced by Singularities that emerge from Gravity bonds.
//!
//! ## Architecture
//! ```text
//! ┌─────────────────────────────────────────┐
//! │           Application / Agent            │
//! │  (POSIX, Agent API, VyMatik Query)       │
//! ├─────────────────────────────────────────┤
//! │    Prism Layer (Multiple Projections)    │
//! │  POSIX │ Agent Native │ Database         │
//! ├─────────────────────────────────────────┤
//! │         Particle Store + Gravity         │
//! ├─────────────────────────────────────────┤
//! │  Journal │ CoW │ Deduplication │ Decay   │
//! ├─────────────────────────────────────────┤
//! │  Block Layer: Allocator │ Compression    │
//! ├─────────────────────────────────────────┤
//! │           Disk / Block Device            │
//! └─────────────────────────────────────────┘
//! ```
//!
//! ## Integration Modes
//! - `kernel` feature: Embedded in AuraOS kernel (no_std)
//! - `fuse` feature: FUSE driver for Linux/macOS (std)
//! - `vymatik` feature: VyMatik storage engine backend
//!
//! ## Patents (Suvayar LLC)
//! Multiple novel inventions are patent-pending. See docs/PATENTS.md.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod alloc_bitmap;
pub mod backend;
pub mod btree;
pub mod compress;
pub mod decay;
pub mod dedup;
pub mod embed;
pub mod format;
pub mod fsck;
pub mod hnsw;
pub mod intelligence;
pub mod journal;
pub mod model;
pub mod particle;
pub mod persist;
pub mod prefetch;
pub mod snapshot;
pub mod store;
pub mod stress_tests;
pub mod super_block;
pub mod text;
pub mod vfs;
pub mod volume;
pub mod wal;

// Re-export core types
pub use btree::BTreeNode;
pub use dedup::DedupEngine;
pub use format::{PageHeader, PageType, ParticleRecord, WaveletRecord, crc32};
pub use journal::Journal;
pub use particle::{
    GravityBond, GravityKind, Particle, ParticleId, Resonance, SemanticRole, Singularity, TypeTag,
    Wavelet, WaveletMetadata,
};
pub use super_block::{BLOCK_SIZE, DEFS_MAGIC, FsState, Superblock};
pub use vfs::DefsVfs;
