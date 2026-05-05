//! # DEFS File System Check
//!
//! Volume integrity checker and repair tool.
//!
//! Checks:
//! - Superblock validity
//! - Page header integrity
//! - CRC32 checksums
//! - Orphaned particles
//! - Dangling gravity bonds
//! - Block bitmap consistency
//! - Orphaned blocks
//! - Dedup table consistency
//! - Snapshot table integrity
//! - Particle index consistency

#[cfg(feature = "std")]
mod std_impl {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    use crate::format::{PAGE_SIZE, PageHeader, PageType};
    use crate::particle::ParticleId;
    use crate::persist::PersistentStore;
    use crate::super_block::{BLOCK_SIZE, DEFS_MAGIC, FEAT_COW_SNAPSHOTS, FEAT_DEDUP};
    use crate::volume::{Volume, VolumeError};

    /// Result of a fsck scan
    #[derive(Clone, Debug, Default)]
    pub struct FsckReport {
        pub volume_path: String,
        pub total_blocks: u64,
        pub scanned_blocks: u64,
        pub valid_pages: u64,
        pub corrupted_pages: u64,
        pub orphaned_particles: u64,
        pub dangling_bonds: u64,
        pub bitmap_errors: u64,
        pub orphaned_blocks: u64,
        pub dedup_errors: u64,
        pub snapshot_errors: u64,
        pub index_errors: u64,
        pub repaired: u64,
        pub errors: Vec<String>,
    }

    impl FsckReport {
        pub fn is_clean(&self) -> bool {
            self.corrupted_pages == 0
                && self.orphaned_particles == 0
                && self.dangling_bonds == 0
                && self.bitmap_errors == 0
                && self.orphaned_blocks == 0
                && self.dedup_errors == 0
                && self.snapshot_errors == 0
                && self.index_errors == 0
        }

        pub fn summary(&self) -> String {
            if self.is_clean() {
                format!(
                    "Volume is clean. Scanned {} blocks, {} valid pages.",
                    self.scanned_blocks, self.valid_pages
                )
            } else {
                format!(
                    "Found {} corrupted pages, {} orphaned particles, {} dangling bonds, {} bitmap errors, {} orphaned blocks, {} dedup errors, {} snapshot errors, {} index errors. Repaired {}.",
                    self.corrupted_pages,
                    self.orphaned_particles,
                    self.dangling_bonds,
                    self.bitmap_errors,
                    self.orphaned_blocks,
                    self.dedup_errors,
                    self.snapshot_errors,
                    self.index_errors,
                    self.repaired
                )
            }
        }
    }

    #[derive(Clone, Debug)]
    struct RawSnapshot {
        id: u64,
        _created_at_ns: u64,
        particle_index_block: u64,
        _label: String,
    }

    pub struct FsckEngine {
        pub repair: bool,
    }

    impl FsckEngine {
        pub fn new(repair: bool) -> Self {
            Self { repair }
        }

        pub fn check(&self, path: &Path) -> Result<FsckReport, VolumeError> {
            let mut report = FsckReport {
                volume_path: path.to_string_lossy().to_string(),
                ..Default::default()
            };

            // Open volume
            let mut volume = match Volume::open(path) {
                Ok(v) => v,
                Err(VolumeError::InvalidMagic) => {
                    report.errors.push(format!(
                        "Invalid superblock magic (expected {:016x})",
                        DEFS_MAGIC
                    ));
                    report.corrupted_pages += 1;
                    return Ok(report);
                }
                Err(e) => return Err(e),
            };
            let info = volume.info();
            report.total_blocks = info.total_blocks;

            // Check superblock
            self.check_superblock(&mut volume, &mut report)?;

            // Scan all data blocks
            let bitmap_blocks =
                ((info.total_blocks + 8 * BLOCK_SIZE as u64 - 1) / (8 * BLOCK_SIZE as u64)).max(1);
            let data_start = 1 + bitmap_blocks + info.journal_size as u64;

            for block_num in data_start..info.total_blocks {
                match volume.read_block(block_num) {
                    Ok(block) => {
                        report.scanned_blocks += 1;
                        self.check_block(&block, block_num, &mut report);
                    }
                    Err(_) => {
                        report
                            .errors
                            .push(format!("Failed to read block {}", block_num));
                        report.corrupted_pages += 1;
                    }
                }

                // Limit scan in testing
                if report.scanned_blocks >= 10000 {
                    break;
                }
            }

            // Check bitmap consistency
            self.check_bitmap(&mut volume, data_start, &mut report)?;

            // Semantic checks (particles & bonds)
            self.check_semantic(path, &mut report)?;

            // New structural checks
            self.check_orphaned_blocks(&mut volume, data_start, &mut report)?;
            self.check_particle_index(&mut volume, &mut report)?;

            if volume.has_feature(FEAT_DEDUP) {
                self.check_dedup_consistency(&mut volume, &mut report)?;
            }

            if volume.has_feature(FEAT_COW_SNAPSHOTS) {
                self.check_snapshot_integrity(&mut volume, data_start, &mut report)?;
            }

            Ok(report)
        }

        fn check_semantic(&self, path: &Path, report: &mut FsckReport) -> Result<(), VolumeError> {
            let mut store = match PersistentStore::open(path) {
                Ok(s) => s,
                Err(_) => return Ok(()), // Already reported at volume level
            };

            let _ = store.load_all();

            let all_ids: std::collections::HashSet<ParticleId> = store
                .all_particles()
                .into_iter()
                .map(|p| p.id.clone())
                .collect();

            let mut dangling = 0u64;
            let particles = store.all_particles();
            let particle_clones: Vec<_> = particles.into_iter().cloned().collect();

            for particle in &particle_clones {
                for bond in &particle.gravity {
                    if !all_ids.contains(&bond.target) {
                        dangling += 1;
                        report.errors.push(format!(
                            "Particle {} has dangling bond to {}",
                            particle.id.to_hex(),
                            bond.target.to_hex()
                        ));
                    }
                }
            }

            report.dangling_bonds = dangling;

            // Check for orphaned particles (not referenced by any Contains bond)
            let mut referenced_ids = std::collections::HashSet::new();
            for particle in &particle_clones {
                for bond in &particle.gravity {
                    if bond.kind == crate::particle::GravityKind::Contains {
                        referenced_ids.insert(bond.target.clone());
                    }
                }
            }

            // Find root: directory particle not referenced by any Contains bond
            let unreferenced_dirs: Vec<_> = particle_clones
                .iter()
                .filter(|p| !referenced_ids.contains(&p.id))
                .filter(|p| {
                    p.bonds_by_kind(crate::particle::GravityKind::Contains)
                        .len()
                        > 0
                })
                .collect();
            let root_id = if unreferenced_dirs.len() == 1 {
                Some(unreferenced_dirs[0].id.clone())
            } else {
                None
            };

            let mut orphaned = 0u64;
            for particle in &particle_clones {
                if !referenced_ids.contains(&particle.id) {
                    // Skip the root directory
                    if root_id.as_ref() == Some(&particle.id) {
                        continue;
                    }
                    orphaned += 1;
                    report.errors.push(format!(
                        "Particle {} is orphaned (not in any directory)",
                        particle.id.to_hex()
                    ));
                }
            }
            report.orphaned_particles = orphaned;

            if self.repair && dangling > 0 {
                let mut repaired = 0usize;
                for mut particle in particle_clones {
                    let original_count = particle.gravity.len();
                    particle.gravity.retain(|b| all_ids.contains(&b.target));
                    let removed = original_count - particle.gravity.len();
                    if removed > 0 {
                        store.write(particle).ok();
                        repaired += removed;
                    }
                }
                store.sync().ok();
                report.repaired += repaired as u64;
                if repaired as u64 == dangling {
                    report.dangling_bonds = 0;
                }
                report
                    .errors
                    .push(format!("Repaired {} dangling bonds", repaired));
            }

            Ok(())
        }

        fn check_superblock(
            &self,
            volume: &mut Volume,
            report: &mut FsckReport,
        ) -> Result<(), VolumeError> {
            let block = volume.read_block(0)?;
            let magic = u64::from_le_bytes([
                block[0], block[1], block[2], block[3], block[4], block[5], block[6], block[7],
            ]);

            if magic != DEFS_MAGIC {
                report.errors.push(format!(
                    "Invalid superblock magic: {:016x} (expected {:016x})",
                    magic, DEFS_MAGIC
                ));
                report.corrupted_pages += 1;
            }

            Ok(())
        }

        fn check_block(&self, block: &[u8], block_num: u64, report: &mut FsckReport) {
            // Check if block looks like a page (has valid page type)
            let page_type = block[0];
            if page_type == 0x00 || page_type == PageType::Free as u8 {
                return; // Empty / unallocated block
            }

            let valid_types = [
                PageType::Superblock as u8,
                PageType::Bitmap as u8,
                PageType::Journal as u8,
                PageType::ParticleLeaf as u8,
                PageType::ParticleInternal as u8,
                PageType::ParticleIndex as u8,
                PageType::DimensionColumn as u8,
                PageType::GravityIndex as u8,
                PageType::WaveletPayload as u8,
                PageType::DedupTable as u8,
            ];

            if !valid_types.contains(&page_type) {
                report.errors.push(format!(
                    "Block {} has invalid page type: 0x{:02x}",
                    block_num, page_type
                ));
                report.corrupted_pages += 1;
                return;
            }

            // Verify checksum
            if !PageHeader::verify_block_checksum(block) {
                report
                    .errors
                    .push(format!("Block {} has invalid checksum", block_num));
                report.corrupted_pages += 1;
                return;
            }

            // Verify page header
            let header = PageHeader::from_bytes(&[
                block[0], block[1], block[2], block[3], block[4], block[5], block[6], block[7],
                block[8], block[9], block[10], block[11], block[12], block[13], block[14],
                block[15],
            ]);

            if header.used_bytes > PAGE_SIZE as u16 {
                report.errors.push(format!(
                    "Block {} has invalid used_bytes: {} > {}",
                    block_num, header.used_bytes, PAGE_SIZE
                ));
                report.corrupted_pages += 1;
                return;
            }

            report.valid_pages += 1;
        }

        fn check_bitmap(
            &self,
            volume: &mut Volume,
            data_start: u64,
            report: &mut FsckReport,
        ) -> Result<(), VolumeError> {
            let info = volume.info();
            let total = info.total_blocks;
            let free = info.free_blocks;
            let used = total - free;

            // Simple consistency check: used blocks should not exceed data range
            if used > total {
                report.errors.push(format!(
                    "Bitmap inconsistent: used blocks ({}) > total blocks ({})",
                    used, total
                ));
                report.bitmap_errors += 1;
            }

            // Count allocated blocks in data region
            let mut allocated = 0u64;
            for block in data_start..total.min(data_start + 10000) {
                match volume.read_block(block) {
                    Ok(data) => {
                        if data[0] != 0x00
                            && data[0] != PageType::Free as u8
                            && data.iter().any(|&b| b != 0)
                        {
                            allocated += 1;
                        }
                    }
                    Err(_) => {}
                }
            }

            // Heuristic: allocated blocks should roughly match used count
            // (not exact because of system blocks)
            let expected_used = used.saturating_sub(data_start);
            if allocated > expected_used + 1000 {
                report.errors.push(format!(
                    "Bitmap may under-report usage: found {} active blocks, bitmap says {} used",
                    allocated, expected_used
                ));
                report.bitmap_errors += 1;
            }

            Ok(())
        }

        // ------------------------------------------------------------------
        // Orphaned block detection
        // ------------------------------------------------------------------
        fn check_orphaned_blocks(
            &self,
            volume: &mut Volume,
            data_start: u64,
            report: &mut FsckReport,
        ) -> Result<(), VolumeError> {
            let info = volume.info();
            let total = info.total_blocks;

            // Read bitmap to know which blocks are marked used
            let bitmap_blocks =
                ((total + 8 * BLOCK_SIZE as u64 - 1) / (8 * BLOCK_SIZE as u64)).max(1);
            let mut bitmap = vec![0u8; (bitmap_blocks * BLOCK_SIZE as u64) as usize];
            for i in 0..bitmap_blocks {
                let block_data = volume.read_block(1 + i)?;
                let start = (i * BLOCK_SIZE as u64) as usize;
                let end = ((i + 1) * BLOCK_SIZE as u64) as usize;
                let len = block_data.len().min(end - start);
                bitmap[start..start + len].copy_from_slice(&block_data[..len]);
            }

            // Collect all referenced blocks
            let mut referenced = HashSet::new();

            // Particle index chain itself
            let idx_block = volume.particle_index_block();
            if idx_block != 0 {
                self.collect_chain_blocks(volume, idx_block, &mut referenced)?;
            }

            // Snapshot table chain
            let snap_block = volume.snapshot_table_block();
            if snap_block != 0 {
                self.collect_chain_blocks(volume, snap_block, &mut referenced)?;
            }

            // Dedup table chain
            let dedup_block = volume.dedup_table_block();
            if dedup_block != 0 {
                self.collect_chain_blocks(volume, dedup_block, &mut referenced)?;
            }

            // Particle records and their dimension chains
            if idx_block != 0 {
                let index = self.read_particle_index(volume, idx_block)?;
                for (_id, block_num) in &index {
                    referenced.insert(*block_num);
                    // Read particle to find dimension blocks
                    if let Ok(page) = volume.read_block(*block_num) {
                        if let Some((_particle, dim_pages)) =
                            crate::format::ParticleRecord::deserialize(&page[16..])
                        {
                            for (_name, (page_num, _offset)) in dim_pages {
                                let mut current = page_num as u64;
                                while current != 0 {
                                    referenced.insert(current);
                                    if let Ok(p) = volume.read_block(current) {
                                        if p.len() >= 16 {
                                            let header = PageHeader::from_bytes(&[
                                                p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7],
                                                p[8], p[9], p[10], p[11], p[12], p[13], p[14],
                                                p[15],
                                            ]);
                                            current = header.next_page as u64;
                                        } else {
                                            break;
                                        }
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Snapshot entries also reference particle index blocks
            if snap_block != 0 && volume.has_feature(FEAT_COW_SNAPSHOTS) {
                let snapshots = self.read_snapshot_table_raw(volume, snap_block)?;
                for snap in snapshots {
                    if snap.particle_index_block != 0 {
                        referenced.insert(snap.particle_index_block);
                        // Also collect the chain blocks for this snapshot index
                        let _ = self.collect_chain_blocks(
                            volume,
                            snap.particle_index_block,
                            &mut referenced,
                        );
                    }
                }
            }

            // Dedup entries reference blocks
            if dedup_block != 0 && volume.has_feature(FEAT_DEDUP) {
                let dedups = self.read_dedup_table_raw(volume, dedup_block)?;
                for (_hash, block_num, _ref_count) in dedups {
                    if block_num != 0 {
                        referenced.insert(block_num);
                    }
                }
            }

            // Now scan data region for orphaned blocks
            let mut orphaned = 0u64;
            for block_num in data_start..total.min(data_start + 10000) {
                let byte = (block_num / 8) as usize;
                let bit = (block_num % 8) as u8;
                if byte < bitmap.len() && (bitmap[byte] & (1 << bit)) != 0 {
                    if !referenced.contains(&block_num) {
                        orphaned += 1;
                        report.errors.push(format!(
                            "Orphaned block {}: marked used in bitmap but not referenced",
                            block_num
                        ));
                    }
                }
            }

            report.orphaned_blocks = orphaned;
            Ok(())
        }

        fn collect_chain_blocks(
            &self,
            volume: &mut Volume,
            start_block: u64,
            referenced: &mut HashSet<u64>,
        ) -> Result<(), VolumeError> {
            let mut current = start_block;
            while current != 0 {
                referenced.insert(current);
                let page = volume.read_block(current)?;
                if page.len() < 16 {
                    break;
                }
                if !PageHeader::verify_block_checksum(&page) {
                    break;
                }
                let header = PageHeader::from_bytes(&[
                    page[0], page[1], page[2], page[3], page[4], page[5], page[6], page[7],
                    page[8], page[9], page[10], page[11], page[12], page[13], page[14], page[15],
                ]);
                current = header.next_page as u64;
            }
            Ok(())
        }

        fn read_particle_index(
            &self,
            volume: &mut Volume,
            start_block: u64,
        ) -> Result<Vec<(ParticleId, u64)>, VolumeError> {
            let buf = self.read_chain(volume, start_block)?;
            if buf.len() < 8 {
                return Ok(Vec::new());
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

            let mut index = Vec::new();
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
                index.push((ParticleId(id), block_num));
            }
            Ok(index)
        }

        fn read_chain(
            &self,
            volume: &mut Volume,
            start_block: u64,
        ) -> Result<Vec<u8>, VolumeError> {
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

        // ------------------------------------------------------------------
        // Dedup table consistency
        // ------------------------------------------------------------------
        fn check_dedup_consistency(
            &self,
            volume: &mut Volume,
            report: &mut FsckReport,
        ) -> Result<(), VolumeError> {
            let dedup_block = volume.dedup_table_block();
            if dedup_block == 0 {
                return Ok(());
            }

            let dedups = self.read_dedup_table_raw(volume, dedup_block)?;
            let mut errors = 0u64;

            // Build map from block_num to expected ref_count
            let mut expected_refs: HashMap<u64, u64> = HashMap::new();

            for (_hash, block_num, ref_count) in &dedups {
                // Verify block is valid
                if *block_num == 0 || *block_num >= volume.info().total_blocks {
                    errors += 1;
                    report
                        .errors
                        .push(format!("Dedup entry points to invalid block {}", block_num));
                    continue;
                }

                if *ref_count == 0 {
                    errors += 1;
                    report.errors.push(format!(
                        "Dedup entry for block {} has ref_count=0",
                        block_num
                    ));
                }

                expected_refs.insert(*block_num, *ref_count);
            }

            // Count actual references from particles
            let idx_block = volume.particle_index_block();
            if idx_block != 0 {
                let index = self.read_particle_index(volume, idx_block)?;
                let mut actual_refs: HashMap<u64, u64> = HashMap::new();

                for (_id, block_num) in &index {
                    if let Ok(page) = volume.read_block(*block_num) {
                        if let Some((_particle, dim_pages)) =
                            crate::format::ParticleRecord::deserialize(&page[16..])
                        {
                            for (_name, (page_num, _offset)) in dim_pages {
                                let first_block = page_num as u64;
                                *actual_refs.entry(first_block).or_insert(0) += 1;
                            }
                        }
                    }
                }

                // Compare expected vs actual for dedup blocks
                for (block_num, expected) in &expected_refs {
                    let actual = actual_refs.get(block_num).copied().unwrap_or(0);
                    if actual != *expected {
                        errors += 1;
                        report.errors.push(format!(
                            "Dedup block {} ref_count mismatch: expected {}, actual {}",
                            block_num, expected, actual
                        ));
                    }
                }
            }

            report.dedup_errors = errors;
            Ok(())
        }

        fn read_dedup_table_raw(
            &self,
            volume: &mut Volume,
            start_block: u64,
        ) -> Result<Vec<(ParticleId, u64, u64)>, VolumeError> {
            let buf = self.read_chain(volume, start_block)?;
            if buf.len() < 8 {
                return Ok(Vec::new());
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

            let mut entries = Vec::new();
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
                entries.push((ParticleId(hash), block_num, ref_count));
            }
            Ok(entries)
        }

        // ------------------------------------------------------------------
        // Snapshot table integrity
        // ------------------------------------------------------------------
        fn check_snapshot_integrity(
            &self,
            volume: &mut Volume,
            data_start: u64,
            report: &mut FsckReport,
        ) -> Result<(), VolumeError> {
            let snap_block = volume.snapshot_table_block();
            if snap_block == 0 {
                return Ok(());
            }

            let snapshots = self.read_snapshot_table_raw(volume, snap_block)?;
            let mut errors = 0u64;
            let mut seen_ids = HashSet::new();
            let total = volume.info().total_blocks;

            for snap in &snapshots {
                // Verify snapshot ID uniqueness
                if !seen_ids.insert(snap.id) {
                    errors += 1;
                    report
                        .errors
                        .push(format!("Duplicate snapshot ID: {}", snap.id));
                }

                // Verify particle_index_block is valid
                if snap.particle_index_block != 0 {
                    if snap.particle_index_block < data_start || snap.particle_index_block >= total
                    {
                        errors += 1;
                        report.errors.push(format!(
                            "Snapshot {} particle_index_block {} out of range",
                            snap.id, snap.particle_index_block
                        ));
                    } else if let Ok(page) = volume.read_block(snap.particle_index_block) {
                        if !PageHeader::verify_block_checksum(&page) {
                            errors += 1;
                            report.errors.push(format!(
                                "Snapshot {} particle_index_block {} has invalid checksum",
                                snap.id, snap.particle_index_block
                            ));
                        }
                    } else {
                        errors += 1;
                        report.errors.push(format!(
                            "Snapshot {} particle_index_block {} unreadable",
                            snap.id, snap.particle_index_block
                        ));
                    }
                }
            }

            report.snapshot_errors = errors;
            Ok(())
        }

        fn read_snapshot_table_raw(
            &self,
            volume: &mut Volume,
            start_block: u64,
        ) -> Result<Vec<RawSnapshot>, VolumeError> {
            let buf = self.read_chain(volume, start_block)?;
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

                snapshots.push(RawSnapshot {
                    id,
                    _created_at_ns: created_at_ns,
                    particle_index_block,
                    _label: label,
                });
            }

            Ok(snapshots)
        }

        // ------------------------------------------------------------------
        // Particle index consistency
        // ------------------------------------------------------------------
        fn check_particle_index(
            &self,
            volume: &mut Volume,
            report: &mut FsckReport,
        ) -> Result<(), VolumeError> {
            let idx_block = volume.particle_index_block();
            if idx_block == 0 {
                return Ok(());
            }

            let mut errors = 0u64;
            let mut current = idx_block;

            while current != 0 {
                match volume.read_block(current) {
                    Ok(page) => {
                        // Verify checksum
                        if !PageHeader::verify_block_checksum(&page) {
                            errors += 1;
                            report.errors.push(format!(
                                "Particle index block {} has invalid checksum",
                                current
                            ));
                        }

                        // Verify page type
                        if page.len() >= 16 {
                            let header = PageHeader::from_bytes(&[
                                page[0], page[1], page[2], page[3], page[4], page[5], page[6],
                                page[7], page[8], page[9], page[10], page[11], page[12], page[13],
                                page[14], page[15],
                            ]);
                            if header.page_type != PageType::ParticleIndex as u8 {
                                errors += 1;
                                report.errors.push(format!(
                                    "Particle index block {} has wrong page type: 0x{:02x}",
                                    current, header.page_type
                                ));
                            }
                        }

                        if page.len() >= 16 {
                            let header = PageHeader::from_bytes(&[
                                page[0], page[1], page[2], page[3], page[4], page[5], page[6],
                                page[7], page[8], page[9], page[10], page[11], page[12], page[13],
                                page[14], page[15],
                            ]);
                            current = header.next_page as u64;
                        } else {
                            break;
                        }
                    }
                    Err(_) => {
                        errors += 1;
                        report
                            .errors
                            .push(format!("Particle index block {} unreadable", current));
                        break;
                    }
                }
            }

            report.index_errors = errors;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write;
        use std::path::PathBuf;

        #[test]
        fn test_fsck_clean_volume() {
            let path = PathBuf::from("/tmp/test_fsck_clean.defs");
            let _ = std::fs::remove_file(&path);

            Volume::create(&path, 5, "FsckTest").unwrap();

            let fsck = FsckEngine::new(false);
            let report = fsck.check(&path).unwrap();

            assert!(report.is_clean(), "Report: {:?}", report);
            assert_eq!(report.corrupted_pages, 0);

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_fsck_corrupted_magic() {
            let path = PathBuf::from("/tmp/test_fsck_corrupt.defs");
            let _ = std::fs::remove_file(&path);

            Volume::create(&path, 5, "FsckTest").unwrap();

            // Corrupt magic
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.write_all(b"BAD!").unwrap();
            drop(file);

            let fsck = FsckEngine::new(false);
            let report = fsck.check(&path).unwrap();

            assert!(!report.is_clean());
            assert!(
                report
                    .errors
                    .iter()
                    .any(|e| e.contains("Invalid superblock magic"))
            );

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_fsck_dangling_bonds() {
            let path = PathBuf::from("/tmp/test_fsck_dangling.defs");
            let _ = std::fs::remove_file(&path);

            let mut store = PersistentStore::create(&path, 5, "FsckTest").unwrap();

            let root_id = ParticleId::from_content(b"root");
            let mut root = crate::particle::Particle::new(root_id.clone());

            let id1 = ParticleId::from_content(b"a");
            let id2 = ParticleId::from_content(b"b");
            let id3 = ParticleId::from_content(b"c"); // non-existent target

            let mut p1 = crate::particle::Particle::new(id1.clone());
            p1.add_bond(id2.clone(), crate::particle::GravityKind::RelatedTo, 1.0);
            p1.add_bond(id3, crate::particle::GravityKind::RelatedTo, 0.5);
            store.write(p1).unwrap();

            let p2 = crate::particle::Particle::new(id2.clone());
            store.write(p2).unwrap();

            // Link to root so they're not orphaned
            root.add_bond(id1, crate::particle::GravityKind::Contains, 1.0);
            root.add_bond(id2, crate::particle::GravityKind::Contains, 1.0);
            store.write(root).unwrap();

            store.sync().unwrap();

            // Check without repair
            let fsck = FsckEngine::new(false);
            let report = fsck.check(&path).unwrap();
            assert!(!report.is_clean());
            assert_eq!(report.dangling_bonds, 1);
            assert!(report.errors.iter().any(|e| e.contains("dangling bond")));

            // Check with repair
            let fsck = FsckEngine::new(true);
            let report = fsck.check(&path).unwrap();
            assert_eq!(
                report.dangling_bonds, 0,
                "Dangling bonds not repaired: {:?}",
                report
            );
            assert_eq!(report.repaired, 1);

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_fsck_orphaned_blocks() {
            let path = PathBuf::from("/tmp/test_fsck_orphaned.defs");
            let _ = std::fs::remove_file(&path);

            let mut volume = Volume::create(&path, 5, "FsckTest").unwrap();

            // Manually allocate a block without referencing it anywhere
            let orphaned = volume.alloc_block().unwrap();
            let mut page = vec![0u8; PAGE_SIZE];
            let header = PageHeader::new(PageType::ParticleLeaf);
            page[0..16].copy_from_slice(&header.to_bytes());
            page[16..20].copy_from_slice(b"test");
            PageHeader::set_block_checksum(&mut page);
            volume.write_block(orphaned, &page).unwrap();
            volume.sync().unwrap();

            let fsck = FsckEngine::new(false);
            let report = fsck.check(&path).unwrap();

            assert!(!report.is_clean(), "Report: {:?}", report);
            assert!(
                report.orphaned_blocks >= 1,
                "Expected at least 1 orphaned block, got {}",
                report.orphaned_blocks
            );
            assert!(
                report.errors.iter().any(|e| e.contains("Orphaned block")),
                "Expected orphaned block error"
            );

            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_fsck_dedup_consistency() {
            let path = PathBuf::from("/tmp/test_fsck_dedup.defs");
            let _ = std::fs::remove_file(&path);

            let mut store = PersistentStore::create(&path, 10, "FsckDedup").unwrap();

            // Create root directory
            let root_id = ParticleId::from_content(b"root");
            let mut root = crate::particle::Particle::new(root_id.clone());
            store.write(root.clone()).unwrap();

            let id1 = ParticleId::from_content(b"doc1");
            let mut p1 = crate::particle::Particle::new(id1.clone());
            p1.set_dimension(
                "content",
                crate::particle::Wavelet::from_binary(b"shared data"),
            );
            store.write(p1.clone()).unwrap();

            let id2 = ParticleId::from_content(b"doc2");
            let mut p2 = crate::particle::Particle::new(id2.clone());
            p2.set_dimension(
                "content",
                crate::particle::Wavelet::from_binary(b"shared data"),
            );
            store.write(p2.clone()).unwrap();

            // Link particles to root so they're not orphaned
            root.add_bond(id1, crate::particle::GravityKind::Contains, 1.0);
            root.add_bond(id2, crate::particle::GravityKind::Contains, 1.0);
            store.write(root).unwrap();
            store.sync().unwrap();

            let fsck = FsckEngine::new(false);
            let report = fsck.check(&path).unwrap();

            assert!(report.is_clean(), "Report: {:?}", report);
            assert_eq!(report.dedup_errors, 0);

            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(feature = "std")]
pub use std_impl::*;
