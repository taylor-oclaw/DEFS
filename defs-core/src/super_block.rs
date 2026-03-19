use alloc::vec::Vec;
use alloc::string::String;

pub const DEFS_MAGIC: u64 = 0x4445465346533031;
pub const BLOCK_SIZE: u32 = 4096;

#[derive(Debug, PartialEq)]
pub enum FsState {
    Clean,
    Dirty,
    Error
}

pub struct Superblock {
    pub magic: u64,
    pub version: u32,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub total_inodes: u64,
    pub free_inodes: u64,
    pub block_size: u32,
    pub journal_start: u64,
    pub journal_size: u32,
    pub root_inode: u64,
    pub mount_count: u32,
    pub last_mount_time: u64,
    pub last_write_time: u64,
    pub state: FsState,
    pub label: [u8; 64],
    pub uuid: [u8; 16],
    pub features: u64
}

pub const FEAT_JOURNAL: u64 = 1;
pub const FEAT_EXTENTS: u64 = 2;
pub const FEAT_SEMANTIC_TAGS: u64 = 4;
pub const FEAT_CONTENT_HASH: u64 = 8;
pub const FEAT_COW_SNAPSHOTS: u64 = 16;
pub const FEAT_DEDUP: u64 = 32;
pub const FEAT_MODEL_AWARE: u64 = 64;

impl Superblock {
    pub fn new(total_blocks: u64, label: &[u8]) -> Self {
        let mut sb_label = [0u8; 64];
        let len = label.len().min(64);
        sb_label[..len].copy_from_slice(&label[..len]);

        Superblock {
            magic: DEFS_MAGIC,
            version: 1,
            total_blocks,
            free_blocks: total_blocks,
            total_inodes: 0,
            free_inodes: 0,
            block_size: BLOCK_SIZE,
            journal_start: 0,
            journal_size: 0,
            root_inode: 2, // Assuming inode 1 is reserved for the superblock
            mount_count: 0,
            last_mount_time: 0,
            last_write_time: 0,
            state: FsState::Clean,
            label: sb_label,
            uuid: [0u8; 16], // UUID should be generated in a real implementation
            features: 0,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.magic == DEFS_MAGIC && self.block_size == BLOCK_SIZE
    }

    pub fn has_feature(&self, feature: u64) -> bool {
        self.features & feature != 0
    }

    pub fn set_feature(&mut self, feature: u64) {
        self.features |= feature;
    }

    pub fn block_groups_count(&self) -> u64 {
        (self.total_blocks + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64
    }
}