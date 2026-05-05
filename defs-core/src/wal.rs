//! # Write-Ahead Log (WAL)
//!
//! Append-only journal for crash recovery.
//! Every particle mutation is logged before being applied to in-memory state.
//! On volume open, un-checkpointed entries are replayed.

#[cfg(feature = "std")]
mod std_impl {
    use crate::format::crc32;
    use crate::particle::{Particle, ParticleId};
    use crate::super_block::BLOCK_SIZE;
    use crate::volume::{Volume, VolumeError};

    use alloc::vec::Vec;

    /// WAL entry types
    #[repr(u8)]
    enum WalEntryType {
        ParticleWrite = 0x01,
        ParticleDelete = 0x02,
        Checkpoint = 0xFF,
    }

    /// On-disk WAL entry header (32 bytes)
    #[derive(Clone, Copy, Debug)]
    struct WalHeader {
        entry_type: u8,
        flags: u8,
        _pad: u16,
        crc32: u32,
        sequence: u64,
        data_len: u32,
        _reserved: u32,
        _reserved2: u64,
    }

    impl WalHeader {
        fn to_bytes(&self) -> [u8; 32] {
            let mut buf = [0u8; 32];
            buf[0] = self.entry_type;
            buf[1] = self.flags;
            buf[4..8].copy_from_slice(&self.crc32.to_le_bytes());
            buf[8..16].copy_from_slice(&self.sequence.to_le_bytes());
            buf[16..20].copy_from_slice(&self.data_len.to_le_bytes());
            buf
        }

        fn from_bytes(buf: &[u8; 32]) -> Self {
            Self {
                entry_type: buf[0],
                flags: buf[1],
                _pad: 0,
                crc32: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
                sequence: u64::from_le_bytes([
                    buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
                ]),
                data_len: u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]),
                _reserved: 0,
                _reserved2: 0,
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct WalEntry {
        pub sequence: u64,
        pub particle: Option<Particle>,
        pub deleted_id: Option<ParticleId>,
        pub is_checkpoint: bool,
    }

    /// Write-ahead log using the journal region of a DEFS volume
    pub struct WriteAheadLog {
        journal_start: u64,
        journal_size: u32, // in blocks
        next_sequence: u64,
        write_offset: u64, // byte offset within journal region
        entries_since_checkpoint: u64,
    }

    impl WriteAheadLog {
        pub fn new(journal_start: u64, journal_size: u32) -> Self {
            Self {
                journal_start,
                journal_size,
                next_sequence: 1,
                write_offset: 0,
                entries_since_checkpoint: 0,
            }
        }

        pub fn open(volume: &mut Volume) -> Result<Self, VolumeError> {
            let info = volume.info();
            let journal_start = info.journal_start;
            let journal_size = info.journal_size;

            let mut wal = Self::new(journal_start, journal_size);

            // Scan journal to find the tail and highest sequence
            wal.scan_tail(volume)?;

            Ok(wal)
        }

        /// Scan journal to find where valid entries end and determine next sequence
        fn scan_tail(&mut self, volume: &mut Volume) -> Result<(), VolumeError> {
            let journal_bytes = self.journal_size as u64 * BLOCK_SIZE as u64;
            let mut last_valid_seq = 0u64;
            let mut last_valid_offset = 0u64;
            let mut offset = 0u64;

            while offset + 32 <= journal_bytes {
                let block_num = self.journal_start + (offset / BLOCK_SIZE as u64);
                let block_offset = (offset % BLOCK_SIZE as u64) as usize;

                let _block = vec![0u8; BLOCK_SIZE as usize];
                let block_result = volume.read_block(block_num);
                if block_result.is_err() {
                    break;
                }
                let block = block_result.unwrap();

                if block_offset + 32 > BLOCK_SIZE as usize {
                    offset = ((offset / BLOCK_SIZE as u64) + 1) * BLOCK_SIZE as u64;
                    continue;
                }

                let header_bytes: [u8; 32] =
                    block[block_offset..block_offset + 32].try_into().unwrap();
                let header = WalHeader::from_bytes(&header_bytes);

                if header.entry_type == 0x00 || header.entry_type == 0xFF && header.data_len == 0 {
                    // Empty or looks like free space
                    break;
                }

                if header.entry_type != WalEntryType::ParticleWrite as u8
                    && header.entry_type != WalEntryType::ParticleDelete as u8
                    && header.entry_type != WalEntryType::Checkpoint as u8
                {
                    break;
                }

                let entry_total = 32 + header.data_len as u64;
                if offset + entry_total > journal_bytes {
                    break;
                }

                // Verify CRC32
                let data = Self::read_journal_bytes(
                    volume,
                    self.journal_start,
                    offset + 32,
                    header.data_len as usize,
                )?;
                let computed_crc = crc32(&data);
                let header_crc = header.crc32;

                if computed_crc != header_crc {
                    // Corrupt entry — stop here
                    break;
                }

                last_valid_seq = header.sequence;
                last_valid_offset = offset;
                offset += ((entry_total + 31) / 32) * 32; // align to 32 bytes
                if offset % BLOCK_SIZE as u64 == 0
                    || offset % BLOCK_SIZE as u64 + 32 > BLOCK_SIZE as u64
                {
                    offset =
                        ((offset + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64) * BLOCK_SIZE as u64;
                }
            }

            self.next_sequence = last_valid_seq + 1;
            self.write_offset = last_valid_offset;
            if last_valid_offset > 0 {
                // Recalculate entries since checkpoint by scanning
                self.entries_since_checkpoint = self.count_since_last_checkpoint(volume)?;
            }
            Ok(())
        }

        fn count_since_last_checkpoint(&self, volume: &mut Volume) -> Result<u64, VolumeError> {
            let entries = self.read_all(volume)?;
            let mut count = 0u64;
            for entry in entries.iter().rev() {
                if entry.is_checkpoint {
                    break;
                }
                count += 1;
            }
            Ok(count)
        }

        fn read_journal_bytes(
            volume: &mut Volume,
            journal_start: u64,
            mut offset: u64,
            len: usize,
        ) -> Result<Vec<u8>, VolumeError> {
            let mut result = Vec::with_capacity(len);
            let mut remaining = len;
            while remaining > 0 {
                let block_num = journal_start + (offset / BLOCK_SIZE as u64);
                let block_offset = (offset % BLOCK_SIZE as u64) as usize;
                let to_read = remaining.min(BLOCK_SIZE as usize - block_offset);

                let block = volume.read_block(block_num)?;
                result.extend_from_slice(&block[block_offset..block_offset + to_read]);

                offset += to_read as u64;
                remaining -= to_read;
            }
            Ok(result)
        }

        fn write_journal_bytes(
            &self,
            volume: &mut Volume,
            mut offset: u64,
            data: &[u8],
        ) -> Result<(), VolumeError> {
            let mut remaining = data.len();
            let mut data_offset = 0usize;
            while remaining > 0 {
                let block_num = self.journal_start + (offset / BLOCK_SIZE as u64);
                let block_offset = (offset % BLOCK_SIZE as u64) as usize;
                let to_write = remaining.min(BLOCK_SIZE as usize - block_offset);

                let mut block = volume.read_block(block_num)?;
                block[block_offset..block_offset + to_write]
                    .copy_from_slice(&data[data_offset..data_offset + to_write]);
                volume.write_block(block_num, &block)?;

                offset += to_write as u64;
                data_offset += to_write;
                remaining -= to_write;
            }
            Ok(())
        }

        /// Serialize a particle with inline dimensions for WAL
        fn serialize_particle_wal(particle: &Particle) -> Vec<u8> {
            let mut buf = Vec::with_capacity(1024);
            // ID
            buf.extend_from_slice(&particle.id.0);
            // Timestamps
            buf.extend_from_slice(&particle.created_at_ns.to_le_bytes());
            buf.extend_from_slice(&particle.modified_at_ns.to_le_bytes());
            // Dimension count
            buf.extend_from_slice(&(particle.dimensions.len() as u16).to_le_bytes());
            // Gravity count
            buf.extend_from_slice(&(particle.gravity.len() as u16).to_le_bytes());
            // Flags + pad
            buf.extend_from_slice(&0u32.to_le_bytes());

            // Dimensions inline
            for (name, wavelet) in &particle.dimensions {
                let name_bytes = name.as_bytes();
                buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                let wavelet_bytes = crate::format::WaveletRecord::serialize(wavelet);
                buf.extend_from_slice(&(wavelet_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(name_bytes);
                buf.extend_from_slice(&wavelet_bytes);
            }

            // Gravity bonds inline
            for bond in &particle.gravity {
                buf.extend_from_slice(&bond.target.0);
                buf.push(bond.kind as u8);
                buf.extend_from_slice(&bond.strength.to_le_bytes());
                let label_len = bond.label.as_ref().map(|l| l.len()).unwrap_or(0) as u16;
                buf.extend_from_slice(&label_len.to_le_bytes());
                if let Some(label) = &bond.label {
                    buf.extend_from_slice(label.as_bytes());
                }
            }

            buf
        }

        fn deserialize_particle_wal(data: &[u8]) -> Option<Particle> {
            if data.len() < 48 {
                return None;
            }
            let mut offset = 0usize;

            let mut id = [0u8; 32];
            id.copy_from_slice(&data[offset..offset + 32]);
            offset += 32;

            let created_at_ns = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            offset += 8;
            let modified_at_ns = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            offset += 8;

            let dimension_count =
                u16::from_le_bytes(data[offset..offset + 2].try_into().ok()?) as usize;
            offset += 2;
            let gravity_count =
                u16::from_le_bytes(data[offset..offset + 2].try_into().ok()?) as usize;
            offset += 2;
            offset += 4; // flags + pad

            let mut particle = Particle::new(ParticleId(id));
            particle.created_at_ns = created_at_ns;
            particle.modified_at_ns = modified_at_ns;

            for _ in 0..dimension_count {
                if offset + 6 > data.len() {
                    break;
                }
                let name_len =
                    u16::from_le_bytes(data[offset..offset + 2].try_into().ok()?) as usize;
                offset += 2;
                let wavelet_len =
                    u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
                offset += 4;
                if offset + name_len + wavelet_len > data.len() {
                    break;
                }
                let name = String::from_utf8_lossy(&data[offset..offset + name_len]).to_string();
                offset += name_len;
                let wavelet =
                    crate::format::WaveletRecord::deserialize(&data[offset..offset + wavelet_len])?;
                offset += wavelet_len;
                particle.set_dimension(&name, wavelet);
            }

            for _ in 0..gravity_count {
                if offset + 37 > data.len() {
                    break;
                }
                let mut target = [0u8; 32];
                target.copy_from_slice(&data[offset..offset + 32]);
                offset += 32;
                let kind = data[offset];
                offset += 1;
                let strength = f32::from_le_bytes(data[offset..offset + 4].try_into().ok()?);
                offset += 4;
                let label_len =
                    u16::from_le_bytes(data[offset..offset + 2].try_into().ok()?) as usize;
                offset += 2;
                let label = if label_len > 0 {
                    if offset + label_len > data.len() {
                        break;
                    }
                    let l = String::from_utf8_lossy(&data[offset..offset + label_len]).to_string();
                    offset += label_len;
                    Some(l)
                } else {
                    None
                };
                let kind = crate::particle::GravityKind::from_u8(kind);
                particle.gravity.push(crate::particle::GravityBond {
                    target: ParticleId(target),
                    kind,
                    strength,
                    label,
                });
            }

            Some(particle)
        }

        /// Append a particle write to the WAL
        pub fn log_write(
            &mut self,
            volume: &mut Volume,
            particle: &Particle,
        ) -> Result<(), VolumeError> {
            let data = Self::serialize_particle_wal(particle);

            let header = WalHeader {
                entry_type: WalEntryType::ParticleWrite as u8,
                flags: 0,
                _pad: 0,
                crc32: crc32(&data),
                sequence: self.next_sequence,
                data_len: data.len() as u32,
                _reserved: 0,
                _reserved2: 0,
            };

            let journal_bytes = self.journal_size as u64 * BLOCK_SIZE as u64;
            let entry_size = 32 + data.len() as u64;
            let aligned_size = ((entry_size + 31) / 32) * 32;

            if self.write_offset + aligned_size > journal_bytes {
                // Journal full — force checkpoint first
                return Err(VolumeError::DiskFull);
            }

            self.write_journal_bytes(volume, self.write_offset, &header.to_bytes())?;
            self.write_journal_bytes(volume, self.write_offset + 32, &data)?;

            self.next_sequence += 1;
            self.write_offset += aligned_size;
            self.entries_since_checkpoint += 1;

            // If we're near the end of a block, align to next block
            let block_remaining = BLOCK_SIZE as u64 - (self.write_offset % BLOCK_SIZE as u64);
            if block_remaining < 32 {
                self.write_offset += block_remaining;
            }

            Ok(())
        }

        /// Append a particle delete to the WAL
        pub fn log_delete(
            &mut self,
            volume: &mut Volume,
            id: &ParticleId,
        ) -> Result<(), VolumeError> {
            let data = id.0.to_vec();
            let header = WalHeader {
                entry_type: WalEntryType::ParticleDelete as u8,
                flags: 0,
                _pad: 0,
                crc32: crc32(&data),
                sequence: self.next_sequence,
                data_len: data.len() as u32,
                _reserved: 0,
                _reserved2: 0,
            };

            let journal_bytes = self.journal_size as u64 * BLOCK_SIZE as u64;
            let entry_size = 32 + data.len() as u64;
            let aligned_size = ((entry_size + 31) / 32) * 32;

            if self.write_offset + aligned_size > journal_bytes {
                return Err(VolumeError::DiskFull);
            }

            self.write_journal_bytes(volume, self.write_offset, &header.to_bytes())?;
            self.write_journal_bytes(volume, self.write_offset + 32, &data)?;

            self.next_sequence += 1;
            self.write_offset += aligned_size;
            self.entries_since_checkpoint += 1;

            let block_remaining = BLOCK_SIZE as u64 - (self.write_offset % BLOCK_SIZE as u64);
            if block_remaining < 32 {
                self.write_offset += block_remaining;
            }

            Ok(())
        }

        /// Write a checkpoint marker and reset journal
        pub fn checkpoint(&mut self, volume: &mut Volume) -> Result<(), VolumeError> {
            let header = WalHeader {
                entry_type: WalEntryType::Checkpoint as u8,
                flags: 0,
                _pad: 0,
                crc32: 0,
                sequence: self.next_sequence,
                data_len: 0,
                _reserved: 0,
                _reserved2: 0,
            };

            self.write_journal_bytes(volume, self.write_offset, &header.to_bytes())?;
            self.next_sequence += 1;
            self.write_offset += 32;
            self.entries_since_checkpoint = 0;

            // Sync volume to ensure checkpoint is on disk
            volume.sync()?;

            Ok(())
        }

        /// Read and replay all WAL entries since the last checkpoint
        pub fn recover(&self, volume: &mut Volume) -> Result<Vec<WalEntry>, VolumeError> {
            self.read_all(volume)
        }

        fn read_all(&self, volume: &mut Volume) -> Result<Vec<WalEntry>, VolumeError> {
            let mut entries = Vec::new();
            let journal_bytes = self.journal_size as u64 * BLOCK_SIZE as u64;
            let mut offset = 0u64;
            let mut last_checkpoint_offset = 0u64;

            // First pass: find the last checkpoint
            while offset + 32 <= journal_bytes {
                let block_num = self.journal_start + (offset / BLOCK_SIZE as u64);
                let block_offset = (offset % BLOCK_SIZE as u64) as usize;

                let _block = vec![0u8; BLOCK_SIZE as usize];
                let block_result = volume.read_block(block_num);
                if block_result.is_err() {
                    break;
                }
                let block = block_result.unwrap();

                if block_offset + 32 > BLOCK_SIZE as usize {
                    offset = ((offset / BLOCK_SIZE as u64) + 1) * BLOCK_SIZE as u64;
                    continue;
                }

                let header_bytes: [u8; 32] = match block[block_offset..block_offset + 32].try_into()
                {
                    Ok(b) => b,
                    Err(_) => break,
                };
                let header = WalHeader::from_bytes(&header_bytes);

                if header.entry_type == 0x00 {
                    break;
                }
                if header.entry_type != WalEntryType::ParticleWrite as u8
                    && header.entry_type != WalEntryType::ParticleDelete as u8
                    && header.entry_type != WalEntryType::Checkpoint as u8
                {
                    break;
                }

                let entry_total = 32 + header.data_len as u64;
                let aligned = ((entry_total + 31) / 32) * 32;

                if header.entry_type == WalEntryType::Checkpoint as u8 {
                    last_checkpoint_offset = offset + aligned;
                }

                offset += aligned;
                if offset % BLOCK_SIZE as u64 == 0
                    || offset % BLOCK_SIZE as u64 + 32 > BLOCK_SIZE as u64
                {
                    offset =
                        ((offset + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64) * BLOCK_SIZE as u64;
                }
            }

            // Second pass: read entries after last checkpoint
            offset = last_checkpoint_offset;
            while offset + 32 <= journal_bytes {
                let block_num = self.journal_start + (offset / BLOCK_SIZE as u64);
                let block_offset = (offset % BLOCK_SIZE as u64) as usize;

                let _block = vec![0u8; BLOCK_SIZE as usize];
                let block_result = volume.read_block(block_num);
                if block_result.is_err() {
                    break;
                }
                let block = block_result.unwrap();

                if block_offset + 32 > BLOCK_SIZE as usize {
                    offset = ((offset / BLOCK_SIZE as u64) + 1) * BLOCK_SIZE as u64;
                    continue;
                }

                let header_bytes: [u8; 32] = match block[block_offset..block_offset + 32].try_into()
                {
                    Ok(b) => b,
                    Err(_) => break,
                };
                let header = WalHeader::from_bytes(&header_bytes);

                if header.entry_type == 0x00 {
                    break;
                }
                if header.entry_type != WalEntryType::ParticleWrite as u8
                    && header.entry_type != WalEntryType::ParticleDelete as u8
                    && header.entry_type != WalEntryType::Checkpoint as u8
                {
                    break;
                }

                let entry_total = 32 + header.data_len as u64;
                let aligned = ((entry_total + 31) / 32) * 32;

                let data = Self::read_journal_bytes(
                    volume,
                    self.journal_start,
                    offset + 32,
                    header.data_len as usize,
                )?;
                let computed_crc = crc32(&data);
                if computed_crc != header.crc32 {
                    break;
                }

                match header.entry_type {
                    x if x == WalEntryType::ParticleWrite as u8 => {
                        if let Some(particle) = Self::deserialize_particle_wal(&data) {
                            entries.push(WalEntry {
                                sequence: header.sequence,
                                particle: Some(particle),
                                deleted_id: None,
                                is_checkpoint: false,
                            });
                        }
                    }
                    x if x == WalEntryType::ParticleDelete as u8 => {
                        if data.len() == 32 {
                            let mut id_bytes = [0u8; 32];
                            id_bytes.copy_from_slice(&data);
                            entries.push(WalEntry {
                                sequence: header.sequence,
                                particle: None,
                                deleted_id: Some(ParticleId(id_bytes)),
                                is_checkpoint: false,
                            });
                        }
                    }
                    x if x == WalEntryType::Checkpoint as u8 => {
                        entries.push(WalEntry {
                            sequence: header.sequence,
                            particle: None,
                            deleted_id: None,
                            is_checkpoint: true,
                        });
                    }
                    _ => {}
                }

                offset += aligned;
                if offset % BLOCK_SIZE as u64 == 0
                    || offset % BLOCK_SIZE as u64 + 32 > BLOCK_SIZE as u64
                {
                    offset =
                        ((offset + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64) * BLOCK_SIZE as u64;
                }
            }

            Ok(entries)
        }

        /// Reset the journal by zeroing all journal blocks
        pub fn reset(&mut self, volume: &mut Volume) -> Result<(), VolumeError> {
            let empty_block = vec![0u8; BLOCK_SIZE as usize];
            for i in 0..self.journal_size as u64 {
                volume.write_block(self.journal_start + i, &empty_block)?;
            }
            volume.sync()?;
            self.write_offset = 0;
            self.entries_since_checkpoint = 0;
            Ok(())
        }

        pub fn needs_checkpoint(&self) -> bool {
            self.entries_since_checkpoint >= 100
        }

        pub fn len(&self) -> u64 {
            self.entries_since_checkpoint
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::particle::Wavelet;
        use std::path::PathBuf;

        fn tmp_vol(name: &str) -> PathBuf {
            PathBuf::from(format!("/tmp/test_wal_{}.defs", name))
        }

        fn clean(name: &str) {
            let _ = std::fs::remove_file(tmp_vol(name));
        }

        #[test]
        fn test_wal_write_and_recover() {
            let name = "write_recover";
            clean(name);

            let mut vol = Volume::create(&tmp_vol(name), 10, "WalTest").unwrap();
            let info = vol.info();
            let mut wal = WriteAheadLog::new(info.journal_start, info.journal_size);

            let mut p = Particle::new(ParticleId::from_content(b"test"));
            p.set_dimension("name", Wavelet::from_string("test"));
            wal.log_write(&mut vol, &p).unwrap();

            let entries = wal.recover(&mut vol).unwrap();
            assert_eq!(entries.len(), 1);
            assert!(entries[0].particle.is_some());
            assert_eq!(entries[0].particle.as_ref().unwrap().name(), Some("test"));

            clean(name);
        }

        #[test]
        fn test_wal_checkpoint_clears_entries() {
            let name = "checkpoint";
            clean(name);

            let mut vol = Volume::create(&tmp_vol(name), 10, "WalTest").unwrap();
            let info = vol.info();
            let mut wal = WriteAheadLog::new(info.journal_start, info.journal_size);

            let p = Particle::new(ParticleId::from_content(b"a"));
            wal.log_write(&mut vol, &p).unwrap();
            wal.checkpoint(&mut vol).unwrap();

            let p2 = Particle::new(ParticleId::from_content(b"b"));
            wal.log_write(&mut vol, &p2).unwrap();

            let entries = wal.recover(&mut vol).unwrap();
            // Should only recover entries after checkpoint
            assert_eq!(entries.len(), 1);
            assert_eq!(
                entries[0].particle.as_ref().unwrap().id,
                ParticleId::from_content(b"b")
            );

            clean(name);
        }

        #[test]
        fn test_wal_delete_entry() {
            let name = "delete";
            clean(name);

            let mut vol = Volume::create(&tmp_vol(name), 10, "WalTest").unwrap();
            let info = vol.info();
            let mut wal = WriteAheadLog::new(info.journal_start, info.journal_size);

            let id = ParticleId::from_content(b"delete_me");
            wal.log_delete(&mut vol, &id).unwrap();

            let entries = wal.recover(&mut vol).unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].deleted_id, Some(id));

            clean(name);
        }
    }
}

#[cfg(feature = "std")]
pub use std_impl::*;
