//! # DEFS — Data-Enriched File System
//!
//! An AI-native, content-aware filesystem designed for the age of agents.
//!
//! ## Features
//! - **Journaling** — crash-safe writes with WAL
//! - **Extent-based** — contiguous block allocation
//! - **B-tree directories** — O(log n) lookups
//! - **Content-addressable dedup** — automatic block deduplication
//! - **Semantic tags** — AI-generated metadata per file
//! - **Model-aware storage** — layer-addressable AI model files
//! - **Predictive prefetch** — learns access patterns
//! - **CoW snapshots** — continuous versioning with rollback
//! - **Decay policies** — automatic lifecycle management
//!
//! ## Architecture
//! ```text
//! ┌─────────────────────────────────────────┐
//! │           Application / Agent            │
//! ├─────────────────────────────────────────┤
//! │    VFS Layer (POSIX or Aether native)    │
//! ├─────────────────────────────────────────┤
//! │  Intelligence: Prefetch │ Dedup │ Decay  │
//! ├─────────────────────────────────────────┤
//! │  Storage: Inodes │ Journal │ B-Tree      │
//! ├─────────────────────────────────────────┤
//! │  Block Layer: Allocator │ Extents        │
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
//! Multiple novel inventions are patent-pending. See PATENTS.md.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod super_block;
pub mod inode;
pub mod journal;
pub mod btree;
pub mod alloc_bitmap;
pub mod dedup;
pub mod model;
pub mod snapshot;
pub mod prefetch;
pub mod decay;
pub mod vfs;

// Re-export key types
pub use super_block::{Superblock, FsState, DEFS_MAGIC, BLOCK_SIZE};
pub use inode::{Inode, InodeNum, FileType, ContentType};
pub use journal::Journal;
pub use btree::BTreeNode;
pub use dedup::DedupEngine;
pub use vfs::DefsVfs;
