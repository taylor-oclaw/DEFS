use alloc::vec::Vec;
use alloc::string::String;

pub const HASH_SIZE: usize = 32;

#[derive(Clone)]
pub struct ContentHash(pub [u8; HASH_SIZE]);

impl ContentHash {
    /// FNV-1a hash extended to 32 bytes
    pub fn from_data(data: &[u8]) -> Self {
        let mut hash = [0u8; HASH_SIZE];
        // Use 4 different FNV seeds for 32 bytes
        for chunk in 0..4 {
            let mut h: u64 = 0xcbf29ce484222325u64.wrapping_add(chunk as u64 * 0x1234567890abcdef);
            for &b in data {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            let bytes = h.to_le_bytes();
            for i in 0..8 {
                hash[chunk * 8 + i] = bytes[i];
            }
        }
        Self(hash)
    }

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    pub fn matches(&self, other: &ContentHash) -> bool {
        self.0 == other.0
    }
}

pub struct DedupEntry {
    pub hash: ContentHash,
    pub block_num: u64,
    pub ref_count: u32,
    pub size: u32,
}

/// PATENTABLE: Content-addressable dedup engine for DEFS
pub struct DedupEngine {
    entries: Vec<DedupEntry>,
    pub bytes_saved: u64,
    pub total_lookups: u64,
    pub dedup_hits: u64,
}

impl DedupEngine {
    pub fn new() -> Self {
        Self { entries: Vec::new(), bytes_saved: 0, total_lookups: 0, dedup_hits: 0 }
    }

    /// Look up a content hash — if found, return existing block (dedup hit)
    pub fn lookup(&mut self, hash: &ContentHash) -> Option<u64> {
        self.total_lookups += 1;
        for entry in &mut self.entries {
            if entry.hash.matches(hash) {
                self.dedup_hits += 1;
                entry.ref_count += 1;
                return Some(entry.block_num);
            }
        }
        None
    }

    /// Register a new unique block
    pub fn insert(&mut self, hash: ContentHash, block: u64, size: u32) {
        self.entries.push(DedupEntry { hash, block_num: block, ref_count: 1, size });
    }

    /// Store data — dedup if hash exists, allocate new block if not
    pub fn store_or_dedup(&mut self, data: &[u8], block_if_new: u64) -> (u64, bool) {
        let hash = ContentHash::from_data(data);
        if let Some(existing) = self.lookup(&hash) {
            self.bytes_saved += data.len() as u64;
            (existing, true) // deduped
        } else {
            self.insert(hash, block_if_new, data.len() as u32);
            (block_if_new, false) // new block
        }
    }

    /// Release a reference — block freed when ref_count hits 0
    pub fn release(&mut self, hash: &ContentHash) -> bool {
        let mut freed = false;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.hash.matches(hash)) {
            entry.ref_count = entry.ref_count.saturating_sub(1);
            if entry.ref_count == 0 { freed = true; }
        }
        if freed {
            self.entries.retain(|e| e.ref_count > 0);
        }
        freed
    }

    pub fn dedup_ratio(&self) -> f32 {
        if self.total_lookups == 0 { 0.0 }
        else { self.dedup_hits as f32 / self.total_lookups as f32 }
    }

    pub fn unique_blocks(&self) -> usize { self.entries.len() }
    pub fn total_refs(&self) -> u32 { self.entries.iter().map(|e| e.ref_count).sum() }
}
