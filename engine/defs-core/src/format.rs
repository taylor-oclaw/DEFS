//! # DEFS Binary On-Disk Format
//!
//! Phase 1 of DEFS v2: Production-grade persistence.
//!
//! ## Layout
//! ```text
//! Block 0:          Superblock (4 KB)
//! Block 1..N:       Free block bitmap
//! Block J..J+K:     Journal (WAL, circular)
//! Block P..P+M:     Particle table (B+tree pages)
//! Block D..D+E:     Dimension store (columnar pages)
//! Block G..G+F:     Gravity index pages
//! Block W..end:     Wavelet payload blocks (large binaries)
//! ```
//!
//! All multi-byte integers are little-endian.
//! All records are CRC32-checksummed.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::particle::{
    GravityBond, GravityKind, Particle, ParticleId, TypeTag, Wavelet, WaveletMetadata,
};
use crate::super_block::BLOCK_SIZE;

/// Magic bytes for DEFS v2
pub const DEFS2_MAGIC: [u8; 5] = *b"DEFS2";
/// On-disk format version
pub const FORMAT_VERSION: u32 = 1;

/// A 4KB page is the basic I/O unit
pub const PAGE_SIZE: usize = BLOCK_SIZE as usize;

/// Page types
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum PageType {
    Superblock = 0x01,
    Bitmap = 0x02,
    Journal = 0x03,
    ParticleLeaf = 0x10,
    ParticleInternal = 0x11,
    ParticleIndex = 0x12,
    DedupTable = 0x13,
    DimensionColumn = 0x20,
    GravityIndex = 0x30,
    WaveletPayload = 0x40,
    Free = 0xFF,
}

/// Page header (16 bytes) — present at start of every page
#[derive(Clone, Copy, Debug)]
pub struct PageHeader {
    pub page_type: u8,
    pub flags: u8,
    pub checksum: u16,
    pub sequence: u32,
    pub next_page: u32, // 0 = none
    pub used_bytes: u16,
    pub _pad: u16,
}

impl PageHeader {
    pub fn new(page_type: PageType) -> Self {
        Self {
            page_type: page_type as u8,
            flags: 0,
            checksum: 0,
            sequence: 0,
            next_page: 0,
            used_bytes: 16, // header size
            _pad: 0,
        }
    }

    /// Compute CRC32 of block payload (everything after checksum field) and return lower 16 bits
    pub fn compute_checksum(block: &[u8]) -> u16 {
        if block.len() < 4 {
            return 0;
        }
        (crc32(&block[4..]) & 0xFFFF) as u16
    }

    /// Set checksum for a block, mutating the block in place
    pub fn set_block_checksum(block: &mut [u8]) {
        if block.len() < 4 {
            return;
        }
        let checksum = Self::compute_checksum(block);
        block[2..4].copy_from_slice(&checksum.to_le_bytes());
    }

    /// Verify checksum of a block
    pub fn verify_block_checksum(block: &[u8]) -> bool {
        if block.len() < 4 {
            return false;
        }
        let stored = u16::from_le_bytes([block[2], block[3]]);
        let computed = Self::compute_checksum(block);
        stored == computed
    }

    pub fn to_bytes(&self) -> [u8; 16] {
        let mut buf = [0u8; 16];
        buf[0] = self.page_type;
        buf[1] = self.flags;
        buf[2..4].copy_from_slice(&self.checksum.to_le_bytes());
        buf[4..8].copy_from_slice(&self.sequence.to_le_bytes());
        buf[8..12].copy_from_slice(&self.next_page.to_le_bytes());
        buf[12..14].copy_from_slice(&self.used_bytes.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8; 16]) -> Self {
        Self {
            page_type: buf[0],
            flags: buf[1],
            checksum: u16::from_le_bytes([buf[2], buf[3]]),
            sequence: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            next_page: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            used_bytes: u16::from_le_bytes([buf[12], buf[13]]),
            _pad: u16::from_le_bytes([buf[14], buf[15]]),
        }
    }
}

/// On-disk particle record (variable length)
///
/// Layout:
/// ```text
/// [0..32]    particle_id
/// [32..40]   created_at_ns (u64)
/// [40..48]   modified_at_ns (u64)
/// [48..50]   dimension_count (u16)
/// [50..52]   gravity_count (u16)
/// [52..54]   flags (u16)
/// [54..56]   _pad
/// [56..]     dimension_offsets: [(name_len: u16, page: u32, offset: u16); dimension_count]
///            gravity_offsets: [(target_id, kind, strength); gravity_count]
/// ```
pub struct ParticleRecord;

impl ParticleRecord {
    pub const HEADER_SIZE: usize = 56;

    pub fn serialize(
        particle: &Particle,
        dimension_pages: &BTreeMap<String, (u32, u16)>,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);

        // ID
        buf.extend_from_slice(&particle.id.0);
        // Timestamps
        buf.extend_from_slice(&particle.created_at_ns.to_le_bytes());
        buf.extend_from_slice(&particle.modified_at_ns.to_le_bytes());
        // Counts
        buf.extend_from_slice(&(particle.dimensions.len() as u16).to_le_bytes());
        buf.extend_from_slice(&(particle.gravity.len() as u16).to_le_bytes());
        // Flags
        buf.extend_from_slice(&0u16.to_le_bytes());
        // Padding
        buf.extend_from_slice(&0u16.to_le_bytes());

        // Dimension offsets
        for (name, _) in &particle.dimensions {
            let name_bytes = name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            if let Some(&(page, offset)) = dimension_pages.get(name) {
                buf.extend_from_slice(&page.to_le_bytes());
                buf.extend_from_slice(&offset.to_le_bytes());
            } else {
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(&0u16.to_le_bytes());
            }
            buf.extend_from_slice(name_bytes);
        }

        // Gravity bonds (inline for small counts)
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

    pub fn deserialize(data: &[u8]) -> Option<(Particle, BTreeMap<String, (u32, u16)>)> {
        if data.len() < Self::HEADER_SIZE {
            return None;
        }

        let mut offset = 0usize;

        // ID
        let mut id = [0u8; 32];
        id.copy_from_slice(&data[offset..offset + 32]);
        offset += 32;

        let created_at_ns = u64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        offset += 8;

        let modified_at_ns = u64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        offset += 8;

        let dimension_count = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        let gravity_count = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;
        offset += 4; // flags + pad

        let mut particle = Particle::new(ParticleId(id));
        particle.created_at_ns = created_at_ns;
        particle.modified_at_ns = modified_at_ns;

        let mut dimension_pages = BTreeMap::new();

        // Read dimension offsets
        for _ in 0..dimension_count {
            let name_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;
            let page = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            offset += 4;
            let page_offset = u16::from_le_bytes([data[offset], data[offset + 1]]);
            offset += 2;
            let name = String::from_utf8_lossy(&data[offset..offset + name_len]).into_owned();
            offset += name_len;
            dimension_pages.insert(name, (page, page_offset));
        }

        // Read gravity bonds
        for _ in 0..gravity_count {
            let mut target_id = [0u8; 32];
            target_id.copy_from_slice(&data[offset..offset + 32]);
            offset += 32;

            let kind_byte = data[offset];
            offset += 1;
            let kind = match kind_byte {
                0 => GravityKind::Contains,
                1 => GravityKind::DependsOn,
                2 => GravityKind::RelatedTo,
                3 => GravityKind::VersionOf,
                4 => GravityKind::References,
                5 => GravityKind::ComposedOf,
                6 => GravityKind::DerivedFrom,
                _ => GravityKind::Custom,
            };

            let strength = f32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            offset += 4;

            let label_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;
            let label = if label_len > 0 {
                Some(String::from_utf8_lossy(&data[offset..offset + label_len]).into_owned())
            } else {
                None
            };
            offset += label_len;

            particle.gravity.push(GravityBond {
                target: ParticleId(target_id),
                kind,
                strength,
                label,
            });
        }

        Some((particle, dimension_pages))
    }
}

/// Wavelet serialization (variable length)
///
/// Layout:
/// ```text
/// [0]        type_tag (u8)
/// [1..5]     payload_length (u32)
/// [5..9]     metadata_length (u32)
/// [9..]      payload + metadata
/// ```
pub struct WaveletRecord;

impl WaveletRecord {
    pub fn serialize(wavelet: &Wavelet) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(wavelet.tag as u8);

        let payload = &wavelet.payload;
        let meta = WaveletMetadataRecord::serialize(&wavelet.metadata);

        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(meta.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
        buf.extend_from_slice(&meta);

        buf
    }

    pub fn deserialize(data: &[u8]) -> Option<Wavelet> {
        if data.len() < 9 {
            return None;
        }

        let tag = match data[0] {
            0 => TypeTag::Null,
            1 => TypeTag::Bool,
            2 => TypeTag::Int64,
            3 => TypeTag::Float64,
            4 => TypeTag::String,
            5 => TypeTag::Binary,
            6 => TypeTag::Timestamp,
            7 => TypeTag::Vector,
            8 => TypeTag::Semantic,
            9 => TypeTag::Ref,
            10 => TypeTag::Delta,
            _ => TypeTag::Binary,
        };

        let payload_len = u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as usize;
        let meta_len = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as usize;

        let payload_start = 9;
        let meta_start = payload_start + payload_len;

        if data.len() < meta_start + meta_len {
            return None;
        }

        let payload = data[payload_start..meta_start].to_vec();
        let metadata = WaveletMetadataRecord::deserialize(&data[meta_start..meta_start + meta_len])
            .unwrap_or_default();

        Some(Wavelet {
            tag,
            payload,
            metadata,
        })
    }
}

/// WaveletMetadata serialization (fixed 20 bytes)
pub struct WaveletMetadataRecord;

impl WaveletMetadataRecord {
    pub fn serialize(meta: &WaveletMetadata) -> Vec<u8> {
        let mut buf = Vec::with_capacity(20);
        buf.extend_from_slice(&meta.timestamp_ns.to_le_bytes());
        // origin: 32 bytes (or zeros if None)
        if let Some(origin) = &meta.origin {
            buf.extend_from_slice(&origin.0);
        } else {
            buf.extend_from_slice(&[0u8; 32]);
        }
        buf.extend_from_slice(&meta.confidence.to_le_bytes());
        buf.extend_from_slice(&meta.encoding_version.to_le_bytes());
        buf
    }

    pub fn deserialize(data: &[u8]) -> Option<WaveletMetadata> {
        if data.len() < 20 {
            return None;
        }
        let timestamp_ns = u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]);
        let mut origin_id = [0u8; 32];
        if data.len() >= 40 {
            origin_id.copy_from_slice(&data[8..40]);
        }
        let origin = if origin_id.iter().all(|&b| b == 0) {
            None
        } else {
            Some(ParticleId(origin_id))
        };
        let confidence = f32::from_le_bytes([data[40], data[41], data[42], data[43]]);
        let encoding_version = u16::from_le_bytes([data[44], data[45]]);

        Some(WaveletMetadata {
            timestamp_ns,
            origin,
            confidence,
            encoding_version,
        })
    }
}

/// Simple CRC32 checksum (using a small table-based implementation)
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB88320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Verify and update page checksum
pub fn verify_page(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    let stored = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let computed = crc32(&data[4..]);
    stored == computed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::{GravityKind, ParticleId, Wavelet, WaveletMetadata};
    use alloc::collections::BTreeMap;

    #[test]
    fn test_page_header_roundtrip() {
        let h = PageHeader::new(PageType::ParticleLeaf);
        let bytes = h.to_bytes();
        let h2 = PageHeader::from_bytes(&bytes);
        assert_eq!(h.page_type, h2.page_type);
        assert_eq!(h.sequence, h2.sequence);
    }

    #[test]
    fn test_wavelet_record_roundtrip() {
        let w = Wavelet::from_string("hello world");
        let bytes = WaveletRecord::serialize(&w);
        let w2 = WaveletRecord::deserialize(&bytes).unwrap();
        assert_eq!(w.tag, w2.tag);
        assert_eq!(w.payload, w2.payload);
    }

    #[test]
    fn test_wavelet_with_metadata() {
        let mut w = Wavelet::from_int64(42);
        w.metadata = WaveletMetadata {
            timestamp_ns: 1234567890,
            origin: Some(ParticleId::from_content(b"test")),
            confidence: 0.95,
            encoding_version: 1,
        };
        let bytes = WaveletRecord::serialize(&w);
        let w2 = WaveletRecord::deserialize(&bytes).unwrap();
        assert_eq!(w.metadata.timestamp_ns, w2.metadata.timestamp_ns);
        assert_eq!(w.metadata.confidence, w2.metadata.confidence);
    }

    #[test]
    fn test_particle_record_roundtrip() {
        let mut p = Particle::new(ParticleId::from_content(b"test"));
        p.set_dimension("name", Wavelet::from_string("report.pdf"));
        p.set_dimension("content", Wavelet::from_binary(b"file data"));
        p.add_bond(
            ParticleId::from_content(b"other"),
            GravityKind::RelatedTo,
            0.85,
        );
        p.created_at_ns = 1000;
        p.modified_at_ns = 2000;

        let dim_pages: BTreeMap<String, (u32, u16)> = BTreeMap::new();
        let bytes = ParticleRecord::serialize(&p, &dim_pages);
        let (p2, dim_pages2) = ParticleRecord::deserialize(&bytes).unwrap();

        assert_eq!(p.id, p2.id);
        assert_eq!(p.created_at_ns, p2.created_at_ns);
        assert_eq!(p.modified_at_ns, p2.modified_at_ns);
        // Dimensions are stored by-reference in dimension store;
        // particle record only holds offsets
        assert_eq!(p2.dimensions.len(), 0);
        assert_eq!(dim_pages2.len(), p.dimensions.len());
        assert_eq!(p.gravity.len(), p2.gravity.len());
        assert_eq!(p.gravity[0].target, p2.gravity[0].target);
        assert_eq!(p.gravity[0].strength, p2.gravity[0].strength);
    }

    #[test]
    fn test_crc32() {
        let data = b"123456789";
        let checksum = crc32(data);
        // Known CRC32 of "123456789" is 0xCBF43926
        assert_eq!(checksum, 0xCBF43926);
    }
}
