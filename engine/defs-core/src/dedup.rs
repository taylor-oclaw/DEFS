//! # Content-Addressable Deduplication Engine
//!
//! Uses blake3 for cryptographic content hashing.
//! Identical content = same hash = same storage block.

use alloc::vec::Vec;

use crate::particle::ParticleId;

/// A deduplication entry tracking a unique content block
#[derive(Clone)]
pub struct DedupEntry {
    pub hash: ParticleId,
    pub block_num: u64,
    pub ref_count: u32,
    pub size: u32,
}

/// Content-addressable dedup engine for DEFS
pub struct DedupEngine {
    entries: Vec<DedupEntry>,
    pub bytes_saved: u64,
    pub total_lookups: u64,
    pub dedup_hits: u64,
}

impl DedupEngine {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            bytes_saved: 0,
            total_lookups: 0,
            dedup_hits: 0,
        }
    }

    /// Look up a content hash — if found, return existing block (dedup hit)
    pub fn lookup(&mut self, hash: &ParticleId) -> Option<u64> {
        self.total_lookups += 1;
        for entry in &mut self.entries {
            if entry.hash == *hash {
                self.dedup_hits += 1;
                entry.ref_count += 1;
                return Some(entry.block_num);
            }
        }
        None
    }

    /// Register a new unique block
    pub fn insert(&mut self, hash: ParticleId, block: u64, size: u32) {
        self.entries.push(DedupEntry {
            hash,
            block_num: block,
            ref_count: 1,
            size,
        });
    }

    /// Store data — dedup if hash exists, allocate new block if not
    pub fn store_or_dedup(&mut self, data: &[u8], block_if_new: u64) -> (u64, bool) {
        let hash = ParticleId::from_content(data);
        if let Some(existing) = self.lookup(&hash) {
            self.bytes_saved += data.len() as u64;
            (existing, true) // deduped
        } else {
            self.insert(hash, block_if_new, data.len() as u32);
            (block_if_new, false) // new block
        }
    }

    /// Release a reference — block freed when ref_count hits 0
    pub fn release(&mut self, hash: &ParticleId) -> bool {
        let mut freed = false;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.hash == *hash) {
            entry.ref_count = entry.ref_count.saturating_sub(1);
            if entry.ref_count == 0 {
                freed = true;
            }
        }
        if freed {
            self.entries.retain(|e| e.ref_count > 0);
        }
        freed
    }

    pub fn dedup_ratio(&self) -> f32 {
        if self.total_lookups == 0 {
            0.0
        } else {
            self.dedup_hits as f32 / self.total_lookups as f32
        }
    }

    pub fn unique_blocks(&self) -> usize {
        self.entries.len()
    }

    pub fn total_refs(&self) -> u32 {
        self.entries.iter().map(|e| e.ref_count).sum()
    }

    pub fn total_bytes_stored(&self) -> u64 {
        self.entries.iter().map(|e| e.size as u64).sum()
    }

    pub fn total_bytes_referenced(&self) -> u64 {
        self.entries
            .iter()
            .map(|e| (e.size as u64) * (e.ref_count as u64))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dedup_finds_duplicate() {
        let mut engine = DedupEngine::new();
        let data = b"hello world";
        let _hash = ParticleId::from_content(data);

        // First store
        let (block1, was_deduped1) = engine.store_or_dedup(data, 100);
        assert_eq!(block1, 100);
        assert!(!was_deduped1);
        assert_eq!(engine.unique_blocks(), 1);

        // Second store — same data
        let (block2, was_deduped2) = engine.store_or_dedup(data, 200);
        assert_eq!(block2, 100); // points to original block
        assert!(was_deduped2);
        assert_eq!(engine.unique_blocks(), 1); // still only 1 unique block
        assert_eq!(engine.total_refs(), 2);
    }

    #[test]
    fn test_dedup_unique_data() {
        let mut engine = DedupEngine::new();
        let data1 = b"hello";
        let data2 = b"world";

        engine.store_or_dedup(data1, 100);
        engine.store_or_dedup(data2, 200);

        assert_eq!(engine.unique_blocks(), 2);
        assert_eq!(engine.dedup_ratio(), 0.0); // no dedup hits yet
    }

    #[test]
    fn test_release_ref_count() {
        let mut engine = DedupEngine::new();
        let data = b"test data";
        let hash = ParticleId::from_content(data);

        engine.store_or_dedup(data, 100);
        engine.store_or_dedup(data, 200); // deduped, ref_count = 2

        // Release one reference
        let freed = engine.release(&hash);
        assert!(!freed); // ref_count went from 2 to 1, block still alive
        assert_eq!(engine.unique_blocks(), 1);

        // Release last reference
        let freed = engine.release(&hash);
        assert!(freed); // block freed
        assert_eq!(engine.unique_blocks(), 0);
    }

    #[test]
    fn test_bytes_saved_tracking() {
        let mut engine = DedupEngine::new();
        let data = vec![0u8; 1024];

        engine.store_or_dedup(&data, 100);
        engine.store_or_dedup(&data, 200);
        engine.store_or_dedup(&data, 300);

        assert_eq!(engine.bytes_saved, 2048); // 2 dedup hits × 1024 bytes
        assert_eq!(engine.dedup_ratio(), 2.0 / 3.0);
    }
}
