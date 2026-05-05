//! # DEFS Volume Manager
//!
//! Manages the on-disk DEFS volume file.
//! Provides block-level I/O, page allocation, and volume lifecycle.

#[cfg(feature = "std")]
mod std_impl {
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::Path;

    use crate::alloc_bitmap::BlockBitmap;
    use crate::format::FORMAT_VERSION;
    use crate::super_block::{BLOCK_SIZE, DEFS_MAGIC, Superblock};

    /// Default number of blocks to cache in memory
    const BLOCK_CACHE_SIZE: usize = 64;

    /// A DEFS volume on disk
    pub struct Volume {
        file: File,
        superblock: Superblock,
        bitmap: BlockBitmap,
        pub dirty: bool,
        block_cache: HashMap<u64, Vec<u8>>,
        cache_hits: u64,
        cache_misses: u64,
    }

    #[derive(Debug)]
    pub enum VolumeError {
        IoError(String),
        InvalidMagic,
        Corrupted(String),
        DiskFull,
    }

    impl std::fmt::Display for VolumeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                VolumeError::IoError(s) => write!(f, "IO error: {}", s),
                VolumeError::InvalidMagic => write!(f, "Invalid magic number"),
                VolumeError::Corrupted(s) => write!(f, "Corrupted: {}", s),
                VolumeError::DiskFull => write!(f, "Disk full"),
            }
        }
    }

    impl std::error::Error for VolumeError {}

    impl Volume {
        /// Create a new DEFS volume file
        pub fn create(path: &Path, size_mb: u64, label: &str) -> Result<Self, VolumeError> {
            let total_blocks = (size_mb * 1024 * 1024) / (BLOCK_SIZE as u64);
            if total_blocks < 1024 {
                return Err(VolumeError::Corrupted("Volume too small (min 4MB)".into()));
            }

            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            // Pre-allocate file
            let file_size = total_blocks * (BLOCK_SIZE as u64);
            file.set_len(file_size)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            // Write superblock
            let mut superblock = Superblock::new(total_blocks, label.as_bytes());
            superblock.version = FORMAT_VERSION;
            superblock.features = 0xFF; // all features enabled
            superblock.encoding_version = 1;
            superblock.root_singularity = 1;

            // Layout:
            // Block 0: Superblock
            // Block 1..bitmap_blocks: Bitmap
            // Block journal_start..journal_end: Journal (128 blocks)
            // Rest: data blocks

            let bitmap_blocks =
                ((total_blocks + 8 * BLOCK_SIZE as u64 - 1) / (8 * BLOCK_SIZE as u64)).max(1);
            // Journal size: up to 4096 blocks (16MB), but never more than 25% of volume
            let journal_blocks = (4096u64).min((total_blocks / 4).max(128));
            let journal_start = 1 + bitmap_blocks;
            let data_start = journal_start + journal_blocks;

            superblock.journal_start = journal_start;
            superblock.journal_size = journal_blocks as u32;

            let mut vol = Self {
                file,
                superblock,
                bitmap: BlockBitmap::new(total_blocks),
                dirty: false,
                block_cache: HashMap::with_capacity(BLOCK_CACHE_SIZE),
                cache_hits: 0,
                cache_misses: 0,
            };

            // Mark system blocks as used
            for b in 0..data_start {
                vol.bitmap.mark_used(b);
            }

            vol.write_superblock()?;
            vol.write_bitmap()?;
            vol.sync()?;

            Ok(vol)
        }

        /// Open an existing volume
        pub fn open(path: &Path) -> Result<Self, VolumeError> {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            let mut superblock_data = vec![0u8; BLOCK_SIZE as usize];
            file.read_exact(&mut superblock_data)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            let superblock = Self::parse_superblock(&superblock_data)?;

            // Read bitmap
            let bitmap_blocks = ((superblock.total_blocks + 8 * BLOCK_SIZE as u64 - 1)
                / (8 * BLOCK_SIZE as u64))
                .max(1);
            let mut bitmap_data = vec![0u8; (bitmap_blocks * BLOCK_SIZE as u64) as usize];
            file.seek(SeekFrom::Start(BLOCK_SIZE as u64))
                .map_err(|e| VolumeError::IoError(e.to_string()))?;
            file.read_exact(&mut bitmap_data)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            let mut bitmap = BlockBitmap::new(superblock.total_blocks);
            let bitmap_bytes = ((superblock.total_blocks + 7) / 8) as usize;
            bitmap.bits[..bitmap_bytes.min(bitmap_data.len())]
                .copy_from_slice(&bitmap_data[..bitmap_bytes.min(bitmap_data.len())]);
            bitmap.recount_free();

            Ok(Self {
                file,
                superblock,
                bitmap,
                dirty: false,
                block_cache: HashMap::with_capacity(BLOCK_CACHE_SIZE),
                cache_hits: 0,
                cache_misses: 0,
            })
        }

        fn parse_superblock(data: &[u8]) -> Result<Superblock, VolumeError> {
            if data.len() < 64 {
                return Err(VolumeError::Corrupted("Superblock too small".into()));
            }

            let magic = u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);

            if magic != DEFS_MAGIC {
                return Err(VolumeError::InvalidMagic);
            }

            let version = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
            let total_blocks = u64::from_le_bytes([
                data[12], data[13], data[14], data[15], data[16], data[17], data[18], data[19],
            ]);
            let block_size = u32::from_le_bytes([data[20], data[21], data[22], data[23]]);

            if block_size != BLOCK_SIZE {
                return Err(VolumeError::Corrupted(format!(
                    "Invalid block size: {} (expected {})",
                    block_size, BLOCK_SIZE
                )));
            }

            let mut label = [0u8; 64];
            label.copy_from_slice(&data[24..88]);

            let mut sb = Superblock::new(total_blocks, &label);
            sb.version = version;
            sb.journal_start = u64::from_le_bytes([
                data[88], data[89], data[90], data[91], data[92], data[93], data[94], data[95],
            ]);
            sb.journal_size = u32::from_le_bytes([data[96], data[97], data[98], data[99]]);
            sb.root_inode = u64::from_le_bytes([
                data[100], data[101], data[102], data[103], data[104], data[105], data[106],
                data[107],
            ]);
            sb.features = u64::from_le_bytes([
                data[108], data[109], data[110], data[111], data[112], data[113], data[114],
                data[115],
            ]);
            sb.encoding_version = u32::from_le_bytes([data[116], data[117], data[118], data[119]]);
            sb.root_singularity = u64::from_le_bytes([
                data[120], data[121], data[122], data[123], data[124], data[125], data[126],
                data[127],
            ]);
            sb.particle_index_block = u64::from_le_bytes([
                data[128], data[129], data[130], data[131], data[132], data[133], data[134],
                data[135],
            ]);
            sb.snapshot_table_block = u64::from_le_bytes([
                data[136], data[137], data[138], data[139], data[140], data[141], data[142],
                data[143],
            ]);
            sb.dedup_table_block = u64::from_le_bytes([
                data[144], data[145], data[146], data[147], data[148], data[149], data[150],
                data[151],
            ]);

            Ok(sb)
        }

        pub fn write_superblock(&mut self) -> Result<(), VolumeError> {
            let mut data = vec![0u8; BLOCK_SIZE as usize];

            data[0..8].copy_from_slice(&self.superblock.magic.to_le_bytes());
            data[8..12].copy_from_slice(&self.superblock.version.to_le_bytes());
            data[12..20].copy_from_slice(&self.superblock.total_blocks.to_le_bytes());
            data[20..24].copy_from_slice(&self.superblock.block_size.to_le_bytes());
            data[24..88].copy_from_slice(&self.superblock.label);
            data[88..96].copy_from_slice(&self.superblock.journal_start.to_le_bytes());
            data[96..100].copy_from_slice(&self.superblock.journal_size.to_le_bytes());
            data[100..108].copy_from_slice(&self.superblock.root_inode.to_le_bytes());
            data[108..116].copy_from_slice(&self.superblock.features.to_le_bytes());
            data[116..120].copy_from_slice(&self.superblock.encoding_version.to_le_bytes());
            data[120..128].copy_from_slice(&self.superblock.root_singularity.to_le_bytes());
            data[128..136].copy_from_slice(&self.superblock.particle_index_block.to_le_bytes());
            data[136..144].copy_from_slice(&self.superblock.snapshot_table_block.to_le_bytes());
            data[144..152].copy_from_slice(&self.superblock.dedup_table_block.to_le_bytes());

            self.file
                .seek(SeekFrom::Start(0))
                .map_err(|e| VolumeError::IoError(e.to_string()))?;
            self.file
                .write_all(&data)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            Ok(())
        }

        fn write_bitmap(&mut self) -> Result<(), VolumeError> {
            self.file
                .seek(SeekFrom::Start(BLOCK_SIZE as u64))
                .map_err(|e| VolumeError::IoError(e.to_string()))?;
            self.file
                .write_all(&self.bitmap.bits)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;
            Ok(())
        }

        /// Read a block from disk (with in-memory cache)
        pub fn read_block(&mut self, block_num: u64) -> Result<Vec<u8>, VolumeError> {
            if block_num >= self.superblock.total_blocks {
                return Err(VolumeError::Corrupted("Block out of range".into()));
            }

            if let Some(cached) = self.block_cache.get(&block_num) {
                self.cache_hits += 1;
                return Ok(cached.clone());
            }

            self.cache_misses += 1;
            let mut buf = vec![0u8; BLOCK_SIZE as usize];
            let offset = block_num * (BLOCK_SIZE as u64);
            self.file
                .seek(SeekFrom::Start(offset))
                .map_err(|e| VolumeError::IoError(e.to_string()))?;
            self.file
                .read_exact(&mut buf)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            // Insert into cache, evicting if at capacity
            if self.block_cache.len() >= BLOCK_CACHE_SIZE {
                if let Some(key) = self.block_cache.keys().next().copied() {
                    self.block_cache.remove(&key);
                }
            }
            self.block_cache.insert(block_num, buf.clone());

            Ok(buf)
        }

        /// Write a block to disk (invalidates cache entry)
        pub fn write_block(&mut self, block_num: u64, data: &[u8]) -> Result<(), VolumeError> {
            if block_num >= self.superblock.total_blocks {
                return Err(VolumeError::Corrupted("Block out of range".into()));
            }
            if data.len() != BLOCK_SIZE as usize {
                return Err(VolumeError::Corrupted(format!(
                    "Block size mismatch: {} != {}",
                    data.len(),
                    BLOCK_SIZE
                )));
            }

            let offset = block_num * (BLOCK_SIZE as u64);
            self.file
                .seek(SeekFrom::Start(offset))
                .map_err(|e| VolumeError::IoError(e.to_string()))?;
            self.file
                .write_all(data)
                .map_err(|e| VolumeError::IoError(e.to_string()))?;

            // Update cache with written data
            if self.block_cache.len() >= BLOCK_CACHE_SIZE
                && !self.block_cache.contains_key(&block_num)
            {
                if let Some(key) = self.block_cache.keys().next().copied() {
                    self.block_cache.remove(&key);
                }
            }
            self.block_cache.insert(block_num, data.to_vec());

            self.dirty = true;
            Ok(())
        }

        /// Allocate a free block
        pub fn alloc_block(&mut self) -> Result<u64, VolumeError> {
            match self.bitmap.alloc_one() {
                Some(block) => {
                    self.dirty = true;
                    Ok(block)
                }
                None => Err(VolumeError::DiskFull),
            }
        }

        /// Free a block
        pub fn free_block(&mut self, block: u64) -> Result<(), VolumeError> {
            self.bitmap.mark_free(block);
            self.dirty = true;
            Ok(())
        }

        /// Get volume info
        pub fn particle_index_block(&self) -> u64 {
            self.superblock.particle_index_block
        }

        pub fn set_particle_index_block(&mut self, block: u64) {
            self.superblock.particle_index_block = block;
            self.dirty = true;
        }

        pub fn snapshot_table_block(&self) -> u64 {
            self.superblock.snapshot_table_block
        }

        pub fn set_snapshot_table_block(&mut self, block: u64) {
            self.superblock.snapshot_table_block = block;
            self.dirty = true;
        }

        pub fn dedup_table_block(&self) -> u64 {
            self.superblock.dedup_table_block
        }

        pub fn set_dedup_table_block(&mut self, block: u64) {
            self.superblock.dedup_table_block = block;
            self.dirty = true;
        }

        pub fn set_feature(&mut self, feature: u64) {
            self.superblock.features |= feature;
            self.dirty = true;
        }

        pub fn has_feature(&self, feature: u64) -> bool {
            self.superblock.features & feature != 0
        }

        pub fn info(&self) -> VolumeInfo {
            VolumeInfo {
                total_blocks: self.superblock.total_blocks,
                free_blocks: self.bitmap.free_count,
                block_size: BLOCK_SIZE,
                used_percent: ((self.superblock.total_blocks - self.bitmap.free_count) * 100
                    / self.superblock.total_blocks) as u8,
                label: String::from_utf8_lossy(&self.superblock.label)
                    .trim_end_matches('\0')
                    .to_string(),
                encoding_version: self.superblock.encoding_version,
                journal_start: self.superblock.journal_start,
                journal_size: self.superblock.journal_size,
                cache_hits: self.cache_hits,
                cache_misses: self.cache_misses,
            }
        }

        /// Sync volume to disk
        pub fn sync(&mut self) -> Result<(), VolumeError> {
            if self.dirty {
                self.write_superblock()?;
                self.write_bitmap()?;
                self.file
                    .sync_all()
                    .map_err(|e| VolumeError::IoError(e.to_string()))?;
                self.dirty = false;
            }
            Ok(())
        }
    }

    #[derive(Debug, Clone)]
    pub struct VolumeInfo {
        pub total_blocks: u64,
        pub free_blocks: u64,
        pub block_size: u32,
        pub used_percent: u8,
        pub label: String,
        pub encoding_version: u32,
        pub journal_start: u64,
        pub journal_size: u32,
        pub cache_hits: u64,
        pub cache_misses: u64,
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn test_create_and_open_volume() {
            let path = PathBuf::from("/tmp/test_defs_volume.bin");
            let _ = std::fs::remove_file(&path);

            // Create
            let vol = Volume::create(&path, 10, "TestVol").unwrap();
            let info = vol.info();
            assert_eq!(info.label, "TestVol");
            assert!(info.total_blocks > 0);
            assert!(info.free_blocks < info.total_blocks); // system blocks reserved

            // Open
            let mut vol2 = Volume::open(&path).unwrap();
            let info2 = vol2.info();
            assert_eq!(info2.label, "TestVol");
            assert_eq!(info2.total_blocks, info.total_blocks);

            // Read/write block
            let block_num = vol2.alloc_block().unwrap();
            let data = vec![0xABu8; BLOCK_SIZE as usize];
            vol2.write_block(block_num, &data).unwrap();
            vol2.sync().unwrap();

            let read_back = vol2.read_block(block_num).unwrap();
            assert_eq!(read_back, data);

            // Clean up
            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_volume_block_allocation() {
            let path = PathBuf::from("/tmp/test_defs_alloc.bin");
            let _ = std::fs::remove_file(&path);

            let mut vol = Volume::create(&path, 5, "AllocTest").unwrap();
            let info = vol.info();
            let initial_free = info.free_blocks;

            // Allocate several blocks
            let mut blocks = Vec::new();
            for _ in 0..10 {
                blocks.push(vol.alloc_block().unwrap());
            }

            let info2 = vol.info();
            assert_eq!(info2.free_blocks, initial_free - 10);

            // Free them
            for b in blocks {
                vol.free_block(b).unwrap();
            }

            let info3 = vol.info();
            assert_eq!(info3.free_blocks, initial_free);

            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(feature = "std")]
pub use std_impl::*;
