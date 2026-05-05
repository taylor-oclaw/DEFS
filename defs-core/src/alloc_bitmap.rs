use alloc::vec;
use alloc::vec::Vec;
pub type BlockNum = u64;

pub struct BlockBitmap {
    pub bits: Vec<u8>,
    pub total_blocks: u64,
    pub free_count: u64,
}

impl BlockBitmap {
    pub fn new(total: u64) -> Self {
        let bytes = ((total + 7) / 8) as usize;
        Self {
            bits: vec![0; bytes],
            total_blocks: total,
            free_count: total,
        }
    }

    pub fn is_free(&self, block: BlockNum) -> bool {
        let byte = (block / 8) as usize;
        let bit = (block % 8) as u8;
        byte < self.bits.len() && (self.bits[byte] & (1 << bit)) == 0
    }

    pub fn mark_used(&mut self, block: BlockNum) {
        let byte = (block / 8) as usize;
        let bit = (block % 8) as u8;
        if byte < self.bits.len() && self.is_free(block) {
            self.bits[byte] |= 1 << bit;
            self.free_count -= 1;
        }
    }

    pub fn mark_free(&mut self, block: BlockNum) {
        let byte = (block / 8) as usize;
        let bit = (block % 8) as u8;
        if byte < self.bits.len() && !self.is_free(block) {
            self.bits[byte] &= !(1 << bit);
            self.free_count += 1;
        }
    }

    pub fn alloc_one(&mut self) -> Option<BlockNum> {
        for i in 0..self.total_blocks {
            if self.is_free(i) {
                self.mark_used(i);
                return Some(i);
            }
        }
        None
    }

    pub fn alloc_contiguous(&mut self, count: u32) -> Option<BlockNum> {
        let mut start = 0u64;
        let mut found = 0u32;
        while start + found as u64 <= self.total_blocks {
            if self.is_free(start + found as u64) {
                found += 1;
                if found == count {
                    for i in 0..count {
                        self.mark_used(start + i as u64);
                    }
                    return Some(start);
                }
            } else {
                start = start + found as u64 + 1;
                found = 0;
            }
        }
        None
    }

    pub fn free_range(&mut self, start: BlockNum, count: u32) {
        for i in 0..count {
            self.mark_free(start + i as u64);
        }
    }

    pub fn usage_percent(&self) -> u8 {
        if self.total_blocks == 0 {
            return 0;
        }
        ((self.total_blocks - self.free_count) * 100 / self.total_blocks) as u8
    }

    pub fn count_free(&self) -> u64 {
        self.free_count
    }

    /// Recalculate free_count by scanning all bits.
    /// Call this after loading bitmap data from disk.
    pub fn recount_free(&mut self) {
        let mut free = 0u64;
        for i in 0..self.total_blocks {
            if self.is_free(i) {
                free += 1;
            }
        }
        self.free_count = free;
    }
}
