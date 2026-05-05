//! # Persistent Particle Store
//!
//! Bridges the in-memory ParticleStore with the on-disk Volume.
//! Reads/writes particles using the binary DEFS format.

#[cfg(feature = "std")]
mod std_impl {
    use std::collections::HashMap;
    use std::path::Path;

    use crate::format::{PAGE_SIZE, PageHeader, PageType, ParticleRecord, WaveletRecord};
    use crate::particle::{GravityKind, Particle, ParticleId, Wavelet};
    use crate::store::{ParticleStore, SearchQuery, StoreError};
    use crate::super_block::BLOCK_SIZE;
    use crate::volume::{Volume, VolumeError};
    use crate::wal::WriteAheadLog;
    use alloc::collections::BTreeMap;

    /// A persistent particle store backed by a DEFS volume file
    pub struct PersistentStore {
        store: ParticleStore,
        volume: Volume,
        modified_particles: HashMap<ParticleId, bool>,
        _dimension_page_cache: HashMap<String, Vec<u8>>,
        wal: WriteAheadLog,
        particle_index: BTreeMap<ParticleId, u64>,
        dedup_table: HashMap<[u8; 32], (u64, u64)>,
        is_recovering: bool,
    }

    impl PersistentStore {
        /// Create a new persistent store on a volume file
        pub fn create(path: &Path, size_mb: u64, label: &str) -> Result<Self, VolumeError> {
            let mut volume = Volume::create(path, size_mb, label)?;
            volume.set_feature(crate::super_block::FEAT_DEDUP);
            let wal = WriteAheadLog::open(&mut volume)?;
            Ok(Self {
                store: ParticleStore::new(),
                volume,
                modified_particles: HashMap::new(),
                _dimension_page_cache: HashMap::new(),
                wal,
                particle_index: BTreeMap::new(),
                dedup_table: HashMap::new(),
                is_recovering: false,
            })
        }

        /// Open an existing persistent store
        pub fn open(path: &Path) -> Result<Self, VolumeError> {
            let mut volume = Volume::open(path)?;
            let store = ParticleStore::new();

            // Load persisted metadata before WAL replay so deletes can free blocks
            let particle_index = Self::read_index(&mut volume).unwrap_or_default();
            let dedup_table = if volume.has_feature(crate::super_block::FEAT_DEDUP) {
                Self::read_dedup_table(&mut volume).unwrap_or_default()
            } else {
                HashMap::new()
            };

            let wal = WriteAheadLog::open(&mut volume)?;
            let recovered = wal.recover(&mut volume)?;

            let mut ps = Self {
                store,
                volume,
                modified_particles: HashMap::new(),
                _dimension_page_cache: HashMap::new(),
                wal,
                particle_index,
                dedup_table,
                is_recovering: true,
            };

            for entry in recovered {
                if entry.is_checkpoint {
                    continue;
                }
                if let Some(particle) = entry.particle {
                    let _ = ps.store.write(particle);
                }
                if let Some(id) = entry.deleted_id {
                    let _ = ps.delete(&id);
                }
            }

            ps.is_recovering = false;
            Ok(ps)
        }

        /// Write a particle — log to WAL first, then store in memory
        pub fn write(&mut self, particle: Particle) -> Result<(), StoreError> {
            // Write to WAL first for crash safety
            if let Err(_) = self.wal.log_write(&mut self.volume, &particle) {
                // WAL full — sync to free journal space and retry
                self.sync()
                    .map_err(|e| StoreError::IoError(format!("Auto-sync failed: {:?}", e)))?;
                self.wal
                    .log_write(&mut self.volume, &particle)
                    .map_err(|e| StoreError::IoError(format!("WAL error: {:?}", e)))?;
            }
            self.modified_particles.insert(particle.id.clone(), true);
            self.store.write(particle)?;

            // Auto-sync if WAL is getting full
            if self.wal.needs_checkpoint() {
                let _ = self.sync();
            }

            Ok(())
        }

        /// Read a particle from memory (must be loaded first)
        pub fn read(&self, id: &ParticleId) -> Result<Particle, StoreError> {
            self.store.read(id)
        }

        /// Delete a particle — free disk blocks, log to WAL, remove from memory
        pub fn delete(&mut self, id: &ParticleId) -> Result<(), VolumeError> {
            let in_store = self.store.delete(id).is_ok();
            let on_disk = self.particle_index.contains_key(id);

            if !in_store && !on_disk {
                return Err(VolumeError::Corrupted("Particle not found".into()));
            }

            // If on disk, free blocks
            if let Some(&block_num) = self.particle_index.get(id) {
                if let Ok(page) = self.volume.read_block(block_num) {
                    if let Some((_particle, dim_pages)) = ParticleRecord::deserialize(&page[16..]) {
                        for (_name, (page_num, _offset)) in dim_pages {
                            self.free_dimension_block(page_num as u64)?;
                        }
                    }
                }
                self.volume.free_block(block_num)?;
            }

            self.particle_index.remove(id);
            self.modified_particles.remove(id);

            if !self.is_recovering {
                self.wal.log_delete(&mut self.volume, id)?;
            }
            Ok(())
        }

        /// Load a specific particle from disk into memory using the index
        pub fn load_particle(&mut self, id: &ParticleId) -> Result<Particle, StoreError> {
            if let Ok(p) = self.store.read(id) {
                return Ok(p);
            }

            if let Some(&block_num) = self.particle_index.get(id) {
                if let Ok(page) = self.volume.read_block(block_num) {
                    let header = PageHeader::from_bytes(&[
                        page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                        page[8], page[9], page[10], page[11], page[12], page[13], page[14],
                        page[15],
                    ]);
                    if header.page_type == PageType::ParticleLeaf as u8
                        && PageHeader::verify_block_checksum(&page)
                    {
                        if let Some((particle, dim_pages)) =
                            ParticleRecord::deserialize(&page[16..])
                        {
                            let mut full_particle = particle;
                            for (name, (page_num, offset)) in dim_pages {
                                if let Ok(dim_data) =
                                    self.read_dimension_chain(page_num as u64, offset)
                                {
                                    if let Some(w) = WaveletRecord::deserialize(&dim_data) {
                                        full_particle.set_dimension(&name, w);
                                    }
                                }
                            }
                            let _ = self.store.write(full_particle.clone());
                            return Ok(full_particle);
                        }
                    }
                }
            }

            Err(StoreError::NotFound)
        }

        /// Read a single dimension
        pub fn read_dimension(
            &self,
            id: &ParticleId,
            name: &str,
        ) -> Result<Option<Wavelet>, StoreError> {
            self.store.read_dimension(id, name)
        }

        /// Search particles
        pub fn search(&self, query: &SearchQuery) -> Result<Vec<Particle>, StoreError> {
            self.store.search(query)
        }

        /// Get gravity bonds
        pub fn outgoing_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<crate::particle::GravityBond>, StoreError> {
            self.store.outgoing_bonds(id, kind)
        }

        pub fn incoming_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<(ParticleId, crate::particle::GravityBond)>, StoreError> {
            self.store.incoming_bonds(id, kind)
        }

        /// Sync all modified particles to disk, then checkpoint WAL and update index
        pub fn sync(&mut self) -> Result<(), VolumeError> {
            if !self.modified_particles.is_empty() {
                let ids: Vec<ParticleId> = self.modified_particles.keys().cloned().collect();
                for id in ids {
                    if let Ok(particle) = self.store.read(&id) {
                        let block_num = self.write_particle_to_disk(&particle)?;
                        self.particle_index.insert(particle.id.clone(), block_num);
                    }
                }
                self.modified_particles.clear();
            }

            self.volume.sync()?;

            // Write particle index
            self.write_index()?;

            // Write dedup table
            self.write_dedup_table()?;

            // Checkpoint WAL after successful sync
            self.wal.checkpoint(&mut self.volume)?;
            Ok(())
        }

        /// Write a single particle to disk pages, returning the block number
        fn write_particle_to_disk(&mut self, particle: &Particle) -> Result<u64, VolumeError> {
            // 1. Write dimension wavelets to dimension store pages
            let mut dimension_pages = std::collections::BTreeMap::new();
            for (name, wavelet) in &particle.dimensions {
                let wavelet_bytes = WaveletRecord::serialize(wavelet);
                let (page, offset) = self.write_dimension_data(name, &wavelet_bytes)?;
                dimension_pages.insert(name.clone(), (page, offset));
            }

            // 2. Serialize particle record with dimension offsets
            let particle_bytes = ParticleRecord::serialize(particle, &dimension_pages);

            // 3. Write particle record to a particle page
            let block_num = self.write_particle_data(&particle.id, &particle_bytes)?;

            Ok(block_num)
        }

        fn write_dimension_data(
            &mut self,
            name: &str,
            data: &[u8],
        ) -> Result<(u32, u16), VolumeError> {
            let name_bytes = name.as_bytes();
            let data_start = 16 + 2 + name_bytes.len();

            // Compute content hash for dedup (name + data)
            let mut hash_input = Vec::new();
            hash_input.extend_from_slice(name_bytes);
            hash_input.extend_from_slice(data);
            let hash: [u8; 32] = blake3::hash(&hash_input).into();

            // Check dedup table
            if let Some(&(block_num, ref_count)) = self.dedup_table.get(&hash) {
                self.dedup_table.insert(hash, (block_num, ref_count + 1));
                return Ok((block_num as u32, data_start as u16));
            }

            if data_start + data.len() <= PAGE_SIZE {
                // Single block — keep backward-compatible layout
                let block = self.volume.alloc_block()?;
                let mut page = vec![0u8; PAGE_SIZE];

                let mut header = PageHeader::new(PageType::DimensionColumn);
                header.used_bytes = (data_start + data.len()) as u16;
                page[0..16].copy_from_slice(&header.to_bytes());

                page[16..18].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                page[18..18 + name_bytes.len()].copy_from_slice(name_bytes);
                page[data_start..data_start + data.len()].copy_from_slice(data);

                PageHeader::set_block_checksum(&mut page);
                self.volume.write_block(block, &page)?;
                self.dedup_table.insert(hash, (block, 1));
                return Ok((block as u32, data_start as u16));
            }

            // Multi-block chained dimension
            let first_payload = PAGE_SIZE - data_start;
            let remaining = data.len() - first_payload;
            let extra_payload_per_block = PAGE_SIZE - 16;
            let extra_blocks = (remaining + extra_payload_per_block - 1) / extra_payload_per_block;
            let total_blocks = 1 + extra_blocks;

            let mut blocks = Vec::with_capacity(total_blocks);
            for _ in 0..total_blocks {
                blocks.push(self.volume.alloc_block()?);
            }

            // Write first block with name prefix
            let mut page = vec![0u8; PAGE_SIZE];
            let mut header = PageHeader::new(PageType::DimensionColumn);
            header.next_page = blocks[1] as u32;
            header.used_bytes = PAGE_SIZE as u16;
            page[0..16].copy_from_slice(&header.to_bytes());

            page[16..18].copy_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            page[18..18 + name_bytes.len()].copy_from_slice(name_bytes);
            page[data_start..PAGE_SIZE].copy_from_slice(&data[..first_payload]);
            PageHeader::set_block_checksum(&mut page);
            self.volume.write_block(blocks[0], &page)?;

            // Write continuation blocks with raw wavelet bytes
            let mut data_offset = first_payload;
            for i in 1..total_blocks {
                let mut page = vec![0u8; PAGE_SIZE];
                let next_page = if i + 1 < total_blocks {
                    blocks[i + 1] as u32
                } else {
                    0
                };
                let chunk_end = (data_offset + extra_payload_per_block).min(data.len());
                let chunk_len = chunk_end - data_offset;

                let mut header = PageHeader::new(PageType::DimensionColumn);
                header.next_page = next_page;
                header.used_bytes = (16 + chunk_len) as u16;
                page[0..16].copy_from_slice(&header.to_bytes());
                page[16..16 + chunk_len].copy_from_slice(&data[data_offset..chunk_end]);
                PageHeader::set_block_checksum(&mut page);
                self.volume.write_block(blocks[i], &page)?;
                data_offset = chunk_end;
            }

            self.dedup_table.insert(hash, (blocks[0], 1));
            Ok((blocks[0] as u32, data_start as u16))
        }

        /// Read dimension data by following the next_page chain starting at first_block.
        /// The offset is the byte position in the first block where wavelet data begins.
        fn read_dimension_chain(
            &mut self,
            first_block: u64,
            offset: u16,
        ) -> Result<Vec<u8>, VolumeError> {
            let mut buf = Vec::new();
            let mut current = first_block;
            let mut is_first = true;

            while current != 0 {
                let page = self.volume.read_block(current)?;
                let header = PageHeader::from_bytes(&[
                    page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                    page[8], page[9], page[10], page[11], page[12], page[13], page[14], page[15],
                ]);

                if !PageHeader::verify_block_checksum(&page) {
                    break;
                }

                if is_first {
                    buf.extend_from_slice(&page[offset as usize..]);
                    is_first = false;
                } else {
                    let used = header.used_bytes as usize;
                    if used > 16 {
                        buf.extend_from_slice(&page[16..used]);
                    } else {
                        buf.extend_from_slice(&page[16..]);
                    }
                }

                current = header.next_page as u64;
            }

            Ok(buf)
        }

        fn write_particle_data(
            &mut self,
            _id: &ParticleId,
            data: &[u8],
        ) -> Result<u64, VolumeError> {
            let block = self.volume.alloc_block()?;
            let mut page = vec![0u8; PAGE_SIZE];

            let header = PageHeader::new(PageType::ParticleLeaf);
            page[0..16].copy_from_slice(&header.to_bytes());

            if 16 + data.len() > PAGE_SIZE {
                return Err(VolumeError::Corrupted(
                    "Particle data too large for single page".into(),
                ));
            }

            page[16..16 + data.len()].copy_from_slice(data);
            PageHeader::set_block_checksum(&mut page);
            self.volume.write_block(block, &page)?;

            Ok(block)
        }

        /// Write the particle index to disk using chained blocks
        fn write_index(&mut self) -> Result<(), VolumeError> {
            let mut buf = Vec::new();
            let count = self.particle_index.len() as u64;
            buf.extend_from_slice(&count.to_le_bytes());
            for (id, block_num) in &self.particle_index {
                buf.extend_from_slice(&id.0);
                buf.extend_from_slice(&block_num.to_le_bytes());
            }

            // Free old index chain if exists
            let old_block = self.volume.particle_index_block();
            if old_block != 0 {
                let _ = self.free_index_chain(old_block);
            }

            // Allocate new index chain
            let first_block = self.write_index_chain(&buf, PageType::ParticleIndex)?;
            self.volume.set_particle_index_block(first_block);
            self.volume.write_superblock()?;

            Ok(())
        }

        fn write_index_chain(
            &mut self,
            data: &[u8],
            page_type: PageType,
        ) -> Result<u64, VolumeError> {
            let payload_per_block = PAGE_SIZE - 16; // minus header
            let blocks_needed = (data.len() + payload_per_block - 1) / payload_per_block;
            let mut blocks = Vec::new();

            for _ in 0..blocks_needed {
                blocks.push(self.volume.alloc_block()?);
            }

            for (i, &block_num) in blocks.iter().enumerate() {
                let start = i * payload_per_block;
                let end = ((i + 1) * payload_per_block).min(data.len());
                let chunk = &data[start..end];

                let mut page = vec![0u8; PAGE_SIZE];
                let next_page = if i + 1 < blocks_needed {
                    blocks[i + 1] as u32
                } else {
                    0u32
                };

                let header = PageHeader {
                    page_type: page_type as u8,
                    flags: 0,
                    checksum: 0,
                    sequence: 0,
                    next_page,
                    used_bytes: (16 + chunk.len()) as u16,
                    _pad: 0,
                };
                page[0..16].copy_from_slice(&header.to_bytes());
                page[16..16 + chunk.len()].copy_from_slice(chunk);
                PageHeader::set_block_checksum(&mut page);
                self.volume.write_block(block_num, &page)?;
            }

            Ok(blocks[0])
        }

        fn free_index_chain(&mut self, start_block: u64) -> Result<(), VolumeError> {
            let mut current = start_block;
            while current != 0 {
                let page = self.volume.read_block(current)?;
                let header = PageHeader::from_bytes(&[
                    page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                    page[8], page[9], page[10], page[11], page[12], page[13], page[14], page[15],
                ]);
                let next = header.next_page as u64;
                let _ = self.volume.free_block(current);
                current = next;
            }
            Ok(())
        }

        /// Free a dimension block with dedup ref-counting.
        /// If the block is in the dedup table, decrement ref_count and only free when it reaches 0.
        fn free_dimension_block(&mut self, block_num: u64) -> Result<(), VolumeError> {
            if let Some((&hash, &(bn, count))) = self
                .dedup_table
                .iter()
                .find(|(_, (bn, _))| *bn == block_num)
            {
                if count <= 1 {
                    self.dedup_table.remove(&hash);
                    // Free the entire chain (multi-block dimensions)
                    let mut current = block_num;
                    while current != 0 {
                        let page = self.volume.read_block(current)?;
                        let header = PageHeader::from_bytes(&[
                            page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                            page[8], page[9], page[10], page[11], page[12], page[13], page[14],
                            page[15],
                        ]);
                        let next = header.next_page as u64;
                        self.volume.free_block(current)?;
                        current = next;
                    }
                } else {
                    self.dedup_table.insert(hash, (bn, count - 1));
                }
                Ok(())
            } else {
                // Not deduped — free directly (preserves existing behavior)
                self.volume.free_block(block_num)
            }
        }

        /// Write the dedup table to disk using chained blocks
        fn write_dedup_table(&mut self) -> Result<(), VolumeError> {
            if self.dedup_table.is_empty() {
                let old_block = self.volume.dedup_table_block();
                if old_block != 0 {
                    let _ = self.free_index_chain(old_block);
                    self.volume.set_dedup_table_block(0);
                }
                return Ok(());
            }

            let mut buf = Vec::new();
            let count = self.dedup_table.len() as u64;
            buf.extend_from_slice(&count.to_le_bytes());
            for (hash, (block_num, ref_count)) in &self.dedup_table {
                buf.extend_from_slice(hash);
                buf.extend_from_slice(&block_num.to_le_bytes());
                buf.extend_from_slice(&ref_count.to_le_bytes());
            }

            // Free old chain if exists
            let old_block = self.volume.dedup_table_block();
            if old_block != 0 {
                let _ = self.free_index_chain(old_block);
            }

            let first_block = self.write_index_chain(&buf, PageType::DedupTable)?;
            self.volume.set_dedup_table_block(first_block);
            self.volume.write_superblock()?;
            Ok(())
        }

        /// Read the dedup table from disk (chained blocks)
        fn read_dedup_table(
            volume: &mut Volume,
        ) -> Result<HashMap<[u8; 32], (u64, u64)>, VolumeError> {
            Self::read_dedup_table_at(volume, volume.dedup_table_block())
        }

        fn read_dedup_table_at(
            volume: &mut Volume,
            start_block: u64,
        ) -> Result<HashMap<[u8; 32], (u64, u64)>, VolumeError> {
            let mut current = start_block;
            if current == 0 {
                return Ok(HashMap::new());
            }

            let mut buf = Vec::new();
            while current != 0 {
                let page = volume.read_block(current)?;
                if !PageHeader::verify_block_checksum(&page) {
                    break;
                }
                let header = PageHeader::from_bytes(&[
                    page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                    page[8], page[9], page[10], page[11], page[12], page[13], page[14], page[15],
                ]);
                let used = header.used_bytes as usize;
                if used > 16 {
                    buf.extend_from_slice(&page[16..used]);
                }
                current = header.next_page as u64;
            }

            if buf.len() < 8 {
                return Ok(HashMap::new());
            }

            let mut offset = 0usize;
            let count = u64::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
                buf[offset + 4],
                buf[offset + 5],
                buf[offset + 6],
                buf[offset + 7],
            ]);
            offset += 8;

            let mut table = HashMap::new();
            for _ in 0..count {
                if offset + 48 > buf.len() {
                    break;
                }
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;
                let block_num = u64::from_le_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                    buf[offset + 4],
                    buf[offset + 5],
                    buf[offset + 6],
                    buf[offset + 7],
                ]);
                offset += 8;
                let ref_count = u64::from_le_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                    buf[offset + 4],
                    buf[offset + 5],
                    buf[offset + 6],
                    buf[offset + 7],
                ]);
                offset += 8;
                table.insert(hash, (block_num, ref_count));
            }

            Ok(table)
        }

        /// Read the particle index from disk (chained blocks)
        fn read_index(volume: &mut Volume) -> Result<BTreeMap<ParticleId, u64>, VolumeError> {
            let mut current = volume.particle_index_block();
            if current == 0 {
                return Ok(BTreeMap::new());
            }

            let mut buf = Vec::new();
            while current != 0 {
                let page = volume.read_block(current)?;
                if !PageHeader::verify_block_checksum(&page) {
                    break;
                }
                let header = PageHeader::from_bytes(&[
                    page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                    page[8], page[9], page[10], page[11], page[12], page[13], page[14], page[15],
                ]);
                let used = header.used_bytes as usize;
                if used > 16 {
                    buf.extend_from_slice(&page[16..used]);
                }
                current = header.next_page as u64;
            }

            if buf.len() < 8 {
                return Ok(BTreeMap::new());
            }

            let mut offset = 0usize;
            let count = u64::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
                buf[offset + 4],
                buf[offset + 5],
                buf[offset + 6],
                buf[offset + 7],
            ]);
            offset += 8;

            let mut index = BTreeMap::new();
            for _ in 0..count {
                if offset + 40 > buf.len() {
                    break;
                }
                let mut id = [0u8; 32];
                id.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;
                let block_num = u64::from_le_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                    buf[offset + 4],
                    buf[offset + 5],
                    buf[offset + 6],
                    buf[offset + 7],
                ]);
                offset += 8;
                index.insert(ParticleId(id), block_num);
            }

            Ok(index)
        }

        /// Load all particles from disk (used on open).
        /// If a particle index exists, loads only indexed particles.
        /// Otherwise falls back to a full block scan for backward compatibility.
        pub fn load_all(&mut self) -> Result<usize, VolumeError> {
            if !self.particle_index.is_empty() {
                let mut count = 0;
                let entries: Vec<u64> = self.particle_index.values().copied().collect();
                for block_num in entries {
                    if let Ok(particle) = self.load_particle_at(block_num) {
                        let _ = self.store.write(particle);
                        count += 1;
                    }
                }
                return Ok(count);
            }

            // Fallback: full block scan (old volumes without index)
            let info = self.volume.info();
            let total_blocks = info.total_blocks;
            let bitmap_blocks =
                ((total_blocks + 8 * BLOCK_SIZE as u64 - 1) / (8 * BLOCK_SIZE as u64)).max(1);
            let data_start = 1 + bitmap_blocks + info.journal_size as u64;

            let mut count = 0;

            for block_num in data_start..total_blocks {
                match self.volume.read_block(block_num) {
                    Ok(page) => {
                        let header = PageHeader::from_bytes(&[
                            page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                            page[8], page[9], page[10], page[11], page[12], page[13], page[14],
                            page[15],
                        ]);

                        if header.page_type == PageType::ParticleLeaf as u8 {
                            if !PageHeader::verify_block_checksum(&page) {
                                continue; // Skip corrupt pages during load
                            }
                            if let Some((particle, dim_pages)) =
                                ParticleRecord::deserialize(&page[16..])
                            {
                                // Load dimensions from their pages
                                let mut full_particle = particle;
                                for (name, (page_num, offset)) in dim_pages {
                                    if let Ok(dim_data) =
                                        self.read_dimension_chain(page_num as u64, offset)
                                    {
                                        if let Some(w) = WaveletRecord::deserialize(&dim_data) {
                                            full_particle.set_dimension(&name, w);
                                        }
                                    }
                                }
                                let _ = self.store.write(full_particle);
                                count += 1;
                            }
                        }
                    }
                    Err(_) => continue, // Skip unreadable blocks
                }
            }

            Ok(count)
        }

        pub fn info(&self) -> crate::volume::VolumeInfo {
            self.volume.info()
        }

        pub fn particle_count(&self) -> usize {
            self.store.particle_count()
        }

        pub fn all_particles(&self) -> Vec<&Particle> {
            self.store.all_particles()
        }

        /// Check if a particle exists in memory
        pub fn exists(&self, id: &ParticleId) -> bool {
            self.store.read(id).is_ok()
        }

        /// Scan all particles (clone from memory)
        pub fn scan(&self) -> Result<Vec<Particle>, StoreError> {
            Ok(self.store.all_particles().into_iter().cloned().collect())
        }

        /// Write a dimension to an existing particle
        pub fn write_dimension(
            &mut self,
            id: &ParticleId,
            name: &str,
            wavelet: &Wavelet,
        ) -> Result<(), StoreError> {
            let mut particle = self.store.read(id)?;
            particle.set_dimension(name, wavelet.clone());
            self.write(particle)
        }

        // ------------------------------------------------------------------
        // Compaction / defragmentation
        // ------------------------------------------------------------------

        /// Compact the volume by rewriting all indexed particles to new blocks,
        /// reclaiming leaked/orphaned blocks in the process.
        pub fn compact(&mut self) -> Result<(usize, u64), VolumeError> {
            self.sync()?;

            // Clear dedup table so rewritten particles rebuild it with correct ref counts
            self.dedup_table.clear();

            // Load all particles from the current index
            let mut particles = Vec::new();
            let old_blocks: Vec<u64> = self.particle_index.values().copied().collect();
            for block_num in old_blocks {
                if let Ok(particle) = self.load_particle_at(block_num) {
                    particles.push(particle);
                }
            }

            // Clear all non-system blocks from bitmap
            let info = self.volume.info();
            let journal_end = info.journal_start + info.journal_size as u64;
            let total_before = info.total_blocks - info.free_blocks;

            for block_num in journal_end..info.total_blocks {
                self.volume.free_block(block_num)?;
            }

            // Clear superblock pointers so subsequent writes don't free blocks
            // that may have been reallocated to rewritten particles.
            self.volume.set_particle_index_block(0);
            self.volume.set_dedup_table_block(0);

            // Rewrite all particles to new blocks
            self.particle_index.clear();
            self.modified_particles.clear();
            self.store = ParticleStore::new();

            for particle in particles {
                let block_num = self.write_particle_to_disk(&particle)?;
                self.particle_index.insert(particle.id.clone(), block_num);
                let _ = self.store.write(particle);
            }

            // Write new index and sync
            self.write_index()?;
            self.volume.sync()?;

            let info_after = self.volume.info();
            let total_after = info_after.total_blocks - info_after.free_blocks;
            let reclaimed = total_before.saturating_sub(total_after);

            Ok((self.particle_index.len(), reclaimed))
        }

        // ------------------------------------------------------------------
        // Snapshot support (COW — copy-on-write at the index level)
        // ------------------------------------------------------------------

        /// Create a snapshot of the current volume state.
        /// Returns the snapshot ID.
        pub fn snapshot(&mut self, label: &str) -> Result<u64, VolumeError> {
            // Ensure current state is fully persisted
            self.sync()?;

            // Copy the current particle index to new blocks
            let snapshot_index_block =
                self.clone_index_chain(self.volume.particle_index_block())?;

            // Copy the current dedup table to new blocks if dedup is enabled
            let snapshot_dedup_block = if self.volume.has_feature(crate::super_block::FEAT_DEDUP)
                && self.volume.dedup_table_block() != 0
            {
                self.clone_index_chain(self.volume.dedup_table_block())?
            } else {
                0
            };

            // Assign snapshot ID (monotonic counter from table length + 1)
            let mut snapshots = self.read_snapshot_table()?;
            let id = snapshots.last().map(|s| s.id + 1).unwrap_or(1);

            snapshots.push(SnapshotMeta {
                id,
                label: label.to_string(),
                created_at_ns: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
                particle_index_block: snapshot_index_block,
                dedup_table_block: snapshot_dedup_block,
            });

            // Write updated snapshot table
            self.write_snapshot_table(&snapshots)?;

            // Enable snapshot feature flag
            self.volume
                .set_feature(crate::super_block::FEAT_COW_SNAPSHOTS);
            self.volume.write_superblock()?;
            self.volume.sync()?;

            Ok(id)
        }

        /// Restore the volume to a previous snapshot.
        pub fn restore_snapshot(&mut self, snapshot_id: u64) -> Result<(), VolumeError> {
            let snapshots = self.read_snapshot_table()?;
            let snapshot = snapshots
                .iter()
                .find(|s| s.id == snapshot_id)
                .ok_or_else(|| {
                    VolumeError::Corrupted(format!("Snapshot {} not found", snapshot_id))
                })?;

            // Replace current particle index with snapshot's index
            self.particle_index =
                Self::read_index_at(&mut self.volume, snapshot.particle_index_block)?;

            // Restore dedup table if snapshot captured one
            if snapshot.dedup_table_block != 0 {
                self.dedup_table =
                    Self::read_dedup_table_at(&mut self.volume, snapshot.dedup_table_block)?;
                self.volume
                    .set_dedup_table_block(snapshot.dedup_table_block);
            } else {
                self.dedup_table.clear();
                self.volume.set_dedup_table_block(0);
            }

            // Clear in-memory store and reload from restored index
            self.store = ParticleStore::new();
            let entries: Vec<u64> = self.particle_index.values().copied().collect();
            for block_num in entries {
                if let Ok(particle) = self.load_particle_at(block_num) {
                    let _ = self.store.write(particle);
                }
            }

            self.modified_particles.clear();

            // Write the restored index as current; dedup table already points to snapshot clone
            self.write_index()?;
            self.volume.write_superblock()?;
            self.volume.sync()?;

            Ok(())
        }

        /// List all available snapshots.
        pub fn list_snapshots(&mut self) -> Result<Vec<SnapshotMeta>, VolumeError> {
            self.read_snapshot_table()
        }

        // --- Snapshot table helpers ---

        fn read_snapshot_table(&mut self) -> Result<Vec<SnapshotMeta>, VolumeError> {
            let current = self.volume.snapshot_table_block();
            if current == 0 {
                return Ok(Vec::new());
            }

            let buf = Self::read_chain(&mut self.volume, current)?;
            let mut offset = 0usize;
            let mut snapshots = Vec::new();

            while offset + 24 <= buf.len() {
                let id = u64::from_le_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                    buf[offset + 4],
                    buf[offset + 5],
                    buf[offset + 6],
                    buf[offset + 7],
                ]);
                offset += 8;
                let created_at_ns = u64::from_le_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                    buf[offset + 4],
                    buf[offset + 5],
                    buf[offset + 6],
                    buf[offset + 7],
                ]);
                offset += 8;
                let particle_index_block = u64::from_le_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                    buf[offset + 4],
                    buf[offset + 5],
                    buf[offset + 6],
                    buf[offset + 7],
                ]);
                offset += 8;

                if offset + 2 > buf.len() {
                    break;
                }
                let label_len = u16::from_le_bytes([buf[offset], buf[offset + 1]]) as usize;
                offset += 2;

                if offset + label_len > buf.len() {
                    break;
                }
                let label = String::from_utf8_lossy(&buf[offset..offset + label_len]).into_owned();
                offset += label_len;

                // Backward-compatible: dedup_table_block appended after label in newer format
                let mut dedup_table_block = 0u64;
                if offset + 8 <= buf.len() {
                    dedup_table_block = u64::from_le_bytes([
                        buf[offset],
                        buf[offset + 1],
                        buf[offset + 2],
                        buf[offset + 3],
                        buf[offset + 4],
                        buf[offset + 5],
                        buf[offset + 6],
                        buf[offset + 7],
                    ]);
                    offset += 8;
                }

                snapshots.push(SnapshotMeta {
                    id,
                    label,
                    created_at_ns,
                    particle_index_block,
                    dedup_table_block,
                });
            }

            Ok(snapshots)
        }

        fn write_snapshot_table(&mut self, snapshots: &[SnapshotMeta]) -> Result<(), VolumeError> {
            let mut buf = Vec::new();
            for snap in snapshots {
                buf.extend_from_slice(&snap.id.to_le_bytes());
                buf.extend_from_slice(&snap.created_at_ns.to_le_bytes());
                buf.extend_from_slice(&snap.particle_index_block.to_le_bytes());
                let label_bytes = snap.label.as_bytes();
                buf.extend_from_slice(&(label_bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(label_bytes);
                buf.extend_from_slice(&snap.dedup_table_block.to_le_bytes());
            }

            // Free old table if exists
            let old_block = self.volume.snapshot_table_block();
            if old_block != 0 {
                let _ = self.free_index_chain(old_block);
            }

            let first_block = self.write_index_chain(&buf, PageType::ParticleIndex)?;
            self.volume.set_snapshot_table_block(first_block);
            self.volume.write_superblock()?;
            Ok(())
        }

        fn clone_index_chain(&mut self, start_block: u64) -> Result<u64, VolumeError> {
            if start_block == 0 {
                return Ok(0);
            }
            let data = Self::read_chain(&mut self.volume, start_block)?;
            self.write_index_chain(&data, PageType::ParticleIndex)
        }

        fn read_chain(volume: &mut Volume, start_block: u64) -> Result<Vec<u8>, VolumeError> {
            let mut buf = Vec::new();
            let mut current = start_block;
            while current != 0 {
                let page = volume.read_block(current)?;
                if !PageHeader::verify_block_checksum(&page) {
                    break;
                }
                let header = PageHeader::from_bytes(&[
                    page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                    page[8], page[9], page[10], page[11], page[12], page[13], page[14], page[15],
                ]);
                let used = header.used_bytes as usize;
                if used > 16 {
                    buf.extend_from_slice(&page[16..used]);
                }
                current = header.next_page as u64;
            }
            Ok(buf)
        }

        fn read_index_at(
            volume: &mut Volume,
            start_block: u64,
        ) -> Result<BTreeMap<ParticleId, u64>, VolumeError> {
            if start_block == 0 {
                return Ok(BTreeMap::new());
            }
            let buf = Self::read_chain(volume, start_block)?;
            if buf.len() < 8 {
                return Ok(BTreeMap::new());
            }
            let mut offset = 0usize;
            let count = u64::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
                buf[offset + 4],
                buf[offset + 5],
                buf[offset + 6],
                buf[offset + 7],
            ]);
            offset += 8;
            let mut index = BTreeMap::new();
            for _ in 0..count {
                if offset + 40 > buf.len() {
                    break;
                }
                let mut id = [0u8; 32];
                id.copy_from_slice(&buf[offset..offset + 32]);
                offset += 32;
                let block_num = u64::from_le_bytes([
                    buf[offset],
                    buf[offset + 1],
                    buf[offset + 2],
                    buf[offset + 3],
                    buf[offset + 4],
                    buf[offset + 5],
                    buf[offset + 6],
                    buf[offset + 7],
                ]);
                offset += 8;
                index.insert(ParticleId(id), block_num);
            }
            Ok(index)
        }

        fn load_particle_at(&mut self, block_num: u64) -> Result<Particle, VolumeError> {
            let page = self.volume.read_block(block_num)?;
            let header = PageHeader::from_bytes(&[
                page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7], page[8],
                page[9], page[10], page[11], page[12], page[13], page[14], page[15],
            ]);
            if header.page_type != PageType::ParticleLeaf as u8
                || !PageHeader::verify_block_checksum(&page)
            {
                return Err(VolumeError::Corrupted("Invalid particle page".into()));
            }
            if let Some((particle, dim_pages)) = ParticleRecord::deserialize(&page[16..]) {
                let mut full = particle;
                for (name, (page_num, offset)) in dim_pages {
                    if let Ok(dim_data) = self.read_dimension_chain(page_num as u64, offset) {
                        if let Some(w) = WaveletRecord::deserialize(&dim_data) {
                            full.set_dimension(&name, w);
                        }
                    }
                }
                Ok(full)
            } else {
                Err(VolumeError::Corrupted(
                    "Failed to deserialize particle".into(),
                ))
            }
        }
    }

    /// Metadata for a single snapshot
    #[derive(Clone, Debug)]
    pub struct SnapshotMeta {
        pub id: u64,
        pub label: String,
        pub created_at_ns: u64,
        pub particle_index_block: u64,
        pub dedup_table_block: u64,
    }

    // ------------------------------------------------------------------
    // StorageBackend implementation for PersistentStore
    // ------------------------------------------------------------------
    use crate::backend::{BackendMetrics, StorageBackend, TransactionHandle};

    impl StorageBackend for PersistentStore {
        fn name(&self) -> &str {
            "defs-persistent"
        }

        fn write(&mut self, particle: &Particle) -> Result<ParticleId, StoreError> {
            let id = particle.id.clone();
            self.write(particle.clone())?;
            Ok(id)
        }

        fn read(&self, id: &ParticleId) -> Result<Particle, StoreError> {
            self.read(id)
        }

        fn delete(&mut self, id: &ParticleId) -> Result<(), StoreError> {
            PersistentStore::delete(self, id).map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        fn exists(&self, id: &ParticleId) -> bool {
            self.exists(id)
        }

        fn read_dimension(
            &self,
            id: &ParticleId,
            dimension: &str,
        ) -> Result<Option<Wavelet>, StoreError> {
            self.read_dimension(id, dimension)
        }

        fn write_dimension(
            &mut self,
            id: &ParticleId,
            dimension: &str,
            wavelet: &Wavelet,
        ) -> Result<(), StoreError> {
            self.write_dimension(id, dimension, wavelet)
        }

        fn outgoing_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<crate::particle::GravityBond>, StoreError> {
            self.outgoing_bonds(id, kind)
        }

        fn incoming_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<(ParticleId, crate::particle::GravityBond)>, StoreError> {
            self.incoming_bonds(id, kind)
        }

        fn search(&self, query: &SearchQuery) -> Result<Vec<Particle>, StoreError> {
            self.search(query)
        }

        fn scan(&self) -> Result<Vec<Particle>, StoreError> {
            self.scan()
        }

        fn begin_transaction(&mut self) -> Result<TransactionHandle, StoreError> {
            // Stub: no real transaction isolation yet
            Ok(TransactionHandle(1))
        }

        fn commit(&mut self, _txn: TransactionHandle) -> Result<(), StoreError> {
            self.sync()
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        fn rollback(&mut self, _txn: TransactionHandle) -> Result<(), StoreError> {
            // Stub: would need to track uncommitted changes
            Ok(())
        }

        fn snapshot(&mut self, label: &str) -> Result<u64, StoreError> {
            PersistentStore::snapshot(self, label)
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        fn restore_snapshot(&mut self, snapshot_id: u64) -> Result<(), StoreError> {
            PersistentStore::restore_snapshot(self, snapshot_id)
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        fn sync(&mut self) -> Result<(), StoreError> {
            PersistentStore::sync(self).map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        fn metrics(&self) -> BackendMetrics {
            let info = self.info();
            let total = info.total_blocks * info.block_size as u64;
            let free = info.free_blocks * info.block_size as u64;
            let used = total - free;

            BackendMetrics {
                backend_name: "defs-persistent".into(),
                total_bytes_written: used,
                total_bytes_read: 0,
                total_particles_stored: self.particle_count() as u64,
                total_dimensions_stored: 0,
                total_gravity_bonds: 0,
                avg_write_latency_us: 0,
                avg_read_latency_us: 0,
                avg_search_latency_us: 0,
                dedup_ratio: 0.0,
                compression_ratio: 0.0,
                cache_hit_rate: 0.0,
                disk_usage_bytes: used,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn test_persistent_store_roundtrip() {
            let path = PathBuf::from("/tmp/test_persist.defs");
            let _ = std::fs::remove_file(&path);

            // Create and write
            {
                let mut ps = PersistentStore::create(&path, 10, "TestPersist").unwrap();
                let id = ParticleId::from_content(b"doc1");
                let mut p = Particle::new(id);
                p.set_dimension("name", Wavelet::from_string("report.pdf"));
                p.set_dimension("content", Wavelet::from_binary(b"hello world"));
                p.created_at_ns = 12345;
                ps.write(p).unwrap();
                ps.sync().unwrap();
            }

            // Open and read back
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let count = ps.load_all().unwrap();
                assert_eq!(count, 1);

                let id = ParticleId::from_content(b"doc1");
                let p = ps.read(&id).unwrap();
                assert_eq!(p.name(), Some("report.pdf"));
                assert_eq!(p.content().unwrap().as_binary(), Some(&b"hello world"[..]));
                assert_eq!(p.created_at_ns, 12345);
            }

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_large_dimension_multi_block() {
            let path = PathBuf::from("/tmp/test_persist_large.defs");
            let _ = std::fs::remove_file(&path);

            // Create a particle with a dimension larger than one block
            let large_payload = vec![0xABu8; 5000];
            {
                let mut ps = PersistentStore::create(&path, 10, "TestPersistLarge").unwrap();
                let id = ParticleId::from_content(b"bigdoc");
                let mut p = Particle::new(id);
                p.set_dimension("content", Wavelet::from_binary(&large_payload));
                ps.write(p).unwrap();
                ps.sync().unwrap();
            }

            // Re-open and load all
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let count = ps.load_all().unwrap();
                assert_eq!(count, 1);

                let id = ParticleId::from_content(b"bigdoc");
                let p = ps.read(&id).unwrap();
                let content = p.content().unwrap().as_binary().unwrap();
                assert_eq!(content.len(), large_payload.len());
                assert_eq!(content, &large_payload[..]);
            }

            // Also test load_particle path
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let id = ParticleId::from_content(b"bigdoc");
                let p = ps.load_particle(&id).unwrap();
                let content = p.content().unwrap().as_binary().unwrap();
                assert_eq!(content.len(), large_payload.len());
                assert_eq!(content, &large_payload[..]);
            }

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_persistent_store_delete_reclaims_blocks() {
            let path = PathBuf::from("/tmp/test_persist_delete.defs");
            let _ = std::fs::remove_file(&path);

            let id = ParticleId::from_content(b"doc1");

            // Create, write, sync
            {
                let mut ps = PersistentStore::create(&path, 10, "TestDelete").unwrap();
                let mut p = Particle::new(id);
                p.set_dimension("name", Wavelet::from_string("report.pdf"));
                p.set_dimension("content", Wavelet::from_binary(b"hello world"));
                ps.write(p).unwrap();
                ps.sync().unwrap();
            }

            // Delete, sync
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                ps.delete(&id).unwrap();
                ps.sync().unwrap();
            }

            // Reopen and verify particle is gone
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                assert!(ps.load_particle(&id).is_err());
                assert!(!ps.particle_index.contains_key(&id));
            }

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_snapshot_create_and_restore() {
            let path = PathBuf::from("/tmp/test_persist_snapshot.defs");
            let _ = std::fs::remove_file(&path);

            // Phase 1: create volume, write particle v1, snapshot
            {
                let mut ps = PersistentStore::create(&path, 10, "TestSnapshot").unwrap();
                let id = ParticleId::from_content(b"doc1");
                let mut p = Particle::new(id);
                p.set_dimension("name", Wavelet::from_string("v1"));
                p.set_dimension("content", Wavelet::from_binary(b"hello"));
                ps.write(p).unwrap();
                ps.sync().unwrap();

                let snap_id = ps.snapshot("before-update").unwrap();
                assert_eq!(snap_id, 1);

                // Modify particle after snapshot
                let mut p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                p.set_dimension("name", Wavelet::from_string("v2"));
                p.set_dimension("content", Wavelet::from_binary(b"world"));
                ps.write(p).unwrap();
                ps.sync().unwrap();

                // Verify current state is v2
                let p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(p.name(), Some("v2"));
            }

            // Phase 2: reopen and restore snapshot
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let _ = ps.load_all().unwrap();
                // Verify current state is still v2 after reopen
                let p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(p.name(), Some("v2"));

                // List snapshots
                let snaps = ps.list_snapshots().unwrap();
                assert_eq!(snaps.len(), 1);
                assert_eq!(snaps[0].id, 1);
                assert_eq!(snaps[0].label, "before-update");

                // Restore snapshot
                ps.restore_snapshot(1).unwrap();

                // Verify restored state is v1
                let p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(p.name(), Some("v1"));
                assert_eq!(p.content().unwrap().as_binary(), Some(&b"hello"[..]));
            }

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_compaction_reclaims_leaked_blocks() {
            let path = PathBuf::from("/tmp/test_persist_compact.defs");
            let _ = std::fs::remove_file(&path);

            {
                let mut ps = PersistentStore::create(&path, 10, "TestCompact").unwrap();
                let id = ParticleId::from_content(b"doc1");
                let mut p = Particle::new(id);
                p.set_dimension("name", Wavelet::from_string("original"));
                p.set_dimension("content", Wavelet::from_binary(b"hello"));
                ps.write(p).unwrap();
                ps.sync().unwrap();

                // Modify particle twice without compaction — leaks old blocks
                let mut p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                p.set_dimension("name", Wavelet::from_string("update1"));
                ps.write(p).unwrap();
                ps.sync().unwrap();

                let mut p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                p.set_dimension("name", Wavelet::from_string("update2"));
                ps.write(p).unwrap();
                ps.sync().unwrap();

                let before = ps.info();
                let used_before = before.total_blocks - before.free_blocks;

                // Compact
                let (count, reclaimed) = ps.compact().unwrap();
                assert_eq!(count, 1);
                assert!(reclaimed > 0, "Should reclaim leaked blocks");

                let after = ps.info();
                let used_after = after.total_blocks - after.free_blocks;
                assert!(
                    used_after < used_before,
                    "Should use fewer blocks after compaction"
                );

                // Verify particle still readable after compaction
                let p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(p.name(), Some("update2"));
                assert_eq!(p.content().unwrap().as_binary(), Some(&b"hello"[..]));
            }

            // Reopen and verify
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let _ = ps.load_all().unwrap();
                let p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(p.name(), Some("update2"));
            }

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_dedup_reuses_blocks() {
            let path = PathBuf::from("/tmp/test_persist_dedup.defs");
            let _ = std::fs::remove_file(&path);

            {
                let mut ps = PersistentStore::create(&path, 10, "TestDedup").unwrap();
                let id1 = ParticleId::from_content(b"doc1");
                let mut p1 = Particle::new(id1);
                p1.set_dimension("content", Wavelet::from_binary(b"hello world"));
                ps.write(p1).unwrap();
                ps.sync().unwrap();

                let free_after_first = ps.info().free_blocks;

                let id2 = ParticleId::from_content(b"doc2");
                let mut p2 = Particle::new(id2);
                p2.set_dimension("content", Wavelet::from_binary(b"hello world"));
                ps.write(p2).unwrap();
                ps.sync().unwrap();

                let free_after_second = ps.info().free_blocks;

                // Second particle should only cost 1 block (particle record), not 2 (particle + dimension)
                // because dimension data is deduplicated
                assert_eq!(free_after_first - free_after_second, 1);
            }

            // Reopen and verify both particles are readable
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let count = ps.load_all().unwrap();
                assert_eq!(count, 2);

                let id1 = ParticleId::from_content(b"doc1");
                let p1 = ps.read(&id1).unwrap();
                assert_eq!(p1.content().unwrap().as_binary(), Some(&b"hello world"[..]));

                let id2 = ParticleId::from_content(b"doc2");
                let p2 = ps.read(&id2).unwrap();
                assert_eq!(p2.content().unwrap().as_binary(), Some(&b"hello world"[..]));
            }

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_compact_preserves_dedup() {
            let path = PathBuf::from("/tmp/test_persist_compact_dedup.defs");
            let _ = std::fs::remove_file(&path);

            {
                let mut ps = PersistentStore::create(&path, 10, "TestCompactDedup").unwrap();
                let id1 = ParticleId::from_content(b"doc1");
                let mut p1 = Particle::new(id1);
                p1.set_dimension("content", Wavelet::from_binary(b"shared content"));
                ps.write(p1).unwrap();
                ps.sync().unwrap();

                let id2 = ParticleId::from_content(b"doc2");
                let mut p2 = Particle::new(id2);
                p2.set_dimension("content", Wavelet::from_binary(b"shared content"));
                ps.write(p2).unwrap();
                ps.sync().unwrap();

                // Compact
                let (count, _) = ps.compact().unwrap();
                assert_eq!(count, 2);

                // Verify both readable before reopen
                let r1 = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(
                    r1.content().unwrap().as_binary(),
                    Some(&b"shared content"[..])
                );
                let r2 = ps.read(&ParticleId::from_content(b"doc2")).unwrap();
                assert_eq!(
                    r2.content().unwrap().as_binary(),
                    Some(&b"shared content"[..])
                );
            }

            // Reopen and verify
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let count = ps.load_all().unwrap();
                assert_eq!(count, 2);

                let r1 = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(
                    r1.content().unwrap().as_binary(),
                    Some(&b"shared content"[..])
                );
                let r2 = ps.read(&ParticleId::from_content(b"doc2")).unwrap();
                assert_eq!(
                    r2.content().unwrap().as_binary(),
                    Some(&b"shared content"[..])
                );
            }

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_snapshot_preserves_dedup() {
            let path = PathBuf::from("/tmp/test_persist_snapshot_dedup.defs");
            let _ = std::fs::remove_file(&path);

            {
                let mut ps = PersistentStore::create(&path, 10, "TestSnapshotDedup").unwrap();
                let id1 = ParticleId::from_content(b"doc1");
                let mut p1 = Particle::new(id1);
                p1.set_dimension("content", Wavelet::from_binary(b"deduped data"));
                ps.write(p1).unwrap();
                ps.sync().unwrap();

                let id2 = ParticleId::from_content(b"doc2");
                let mut p2 = Particle::new(id2);
                p2.set_dimension("content", Wavelet::from_binary(b"deduped data"));
                ps.write(p2).unwrap();
                ps.sync().unwrap();

                // Take snapshot while both particles share a deduped dimension
                let snap_id = ps.snapshot("with-dedup").unwrap();

                // Modify one particle so the current state diverges from snapshot
                let mut p = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                p.set_dimension("content", Wavelet::from_binary(b"modified data"));
                ps.write(p).unwrap();
                ps.sync().unwrap();

                // Restore snapshot
                ps.restore_snapshot(snap_id).unwrap();

                // Both particles should be readable with original deduped content
                let r1 = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(
                    r1.content().unwrap().as_binary(),
                    Some(&b"deduped data"[..])
                );
                let r2 = ps.read(&ParticleId::from_content(b"doc2")).unwrap();
                assert_eq!(
                    r2.content().unwrap().as_binary(),
                    Some(&b"deduped data"[..])
                );
            }

            // Reopen and verify again
            {
                let mut ps = PersistentStore::open(&path).unwrap();
                let count = ps.load_all().unwrap();
                assert_eq!(count, 2);

                let r1 = ps.read(&ParticleId::from_content(b"doc1")).unwrap();
                assert_eq!(
                    r1.content().unwrap().as_binary(),
                    Some(&b"deduped data"[..])
                );
                let r2 = ps.read(&ParticleId::from_content(b"doc2")).unwrap();
                assert_eq!(
                    r2.content().unwrap().as_binary(),
                    Some(&b"deduped data"[..])
                );
            }

            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(feature = "std")]
pub use std_impl::*;
