//! # DEFS Particle Model
//!
//! The fundamental storage unit of DEFS v2.
//!
//! A Particle replaces the traditional inode. Every file, directory, symlink,
//! and data fragment is a Particle with typed Dimensions and Gravity bonds.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Content-addressable particle identifier (blake3 hash)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct ParticleId(pub [u8; 32]);

impl ParticleId {
    pub fn from_content(content: &[u8]) -> Self {
        Self(blake3::hash(content).into())
    }

    pub fn null() -> Self {
        Self([0u8; 32])
    }

    pub fn is_null(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for byte in &self.0 {
            use core::fmt::Write;
            let _ = write!(s, "{:02x}", byte);
        }
        s
    }
}

/// A Wavelet is the encoded signal within a Dimension.
/// Inspired by VyMatik's wavelet encoding — compact, typed, self-describing.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct Wavelet {
    /// Type tag determines how payload is interpreted
    pub tag: TypeTag,
    /// Raw encoded payload
    pub payload: Vec<u8>,
    /// Metadata: timestamp, origin, confidence, encoding version
    pub metadata: WaveletMetadata,
}

/// Wavelet type tags — what kind of data this wavelet carries
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub enum TypeTag {
    Null,
    Bool,
    Int64,
    Float64,
    String,
    Binary,
    Timestamp,
    Vector,   // Embedding / float array
    Semantic, // Tagged concept (e.g. "finance", "medical")
    Ref,      // Reference to another ParticleId
    Delta,    // Delta from a base wavelet
}

/// Metadata carried with every wavelet
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct WaveletMetadata {
    /// Unix timestamp (nanoseconds)
    pub timestamp_ns: u64,
    /// Origin particle that created this wavelet
    pub origin: Option<ParticleId>,
    /// Confidence score (0.0–1.0) for AI-generated values
    pub confidence: f32,
    /// Encoding format version
    pub encoding_version: u16,
}

impl Wavelet {
    pub fn null() -> Self {
        Self {
            tag: TypeTag::Null,
            payload: Vec::new(),
            metadata: WaveletMetadata::default(),
        }
    }

    pub fn from_string(s: &str) -> Self {
        Self {
            tag: TypeTag::String,
            payload: s.as_bytes().to_vec(),
            metadata: WaveletMetadata::default(),
        }
    }

    pub fn from_binary(data: &[u8]) -> Self {
        Self {
            tag: TypeTag::Binary,
            payload: data.to_vec(),
            metadata: WaveletMetadata::default(),
        }
    }

    pub fn from_int64(v: i64) -> Self {
        Self {
            tag: TypeTag::Int64,
            payload: v.to_le_bytes().to_vec(),
            metadata: WaveletMetadata::default(),
        }
    }

    pub fn from_float64(v: f64) -> Self {
        Self {
            tag: TypeTag::Float64,
            payload: v.to_le_bytes().to_vec(),
            metadata: WaveletMetadata::default(),
        }
    }

    pub fn from_bool(v: bool) -> Self {
        Self {
            tag: TypeTag::Bool,
            payload: vec![v as u8],
            metadata: WaveletMetadata::default(),
        }
    }

    pub fn from_ref(target: &ParticleId) -> Self {
        Self {
            tag: TypeTag::Ref,
            payload: target.0.to_vec(),
            metadata: WaveletMetadata::default(),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if self.tag == TypeTag::String {
            core::str::from_utf8(&self.payload).ok()
        } else {
            None
        }
    }

    pub fn as_binary(&self) -> Option<&[u8]> {
        if self.tag == TypeTag::Binary {
            Some(&self.payload)
        } else {
            None
        }
    }

    pub fn as_int64(&self) -> Option<i64> {
        if self.tag == TypeTag::Int64 && self.payload.len() == 8 {
            Some(i64::from_le_bytes([
                self.payload[0],
                self.payload[1],
                self.payload[2],
                self.payload[3],
                self.payload[4],
                self.payload[5],
                self.payload[6],
                self.payload[7],
            ]))
        } else {
            None
        }
    }

    pub fn as_particle_ref(&self) -> Option<ParticleId> {
        if self.tag == TypeTag::Ref && self.payload.len() == 32 {
            let mut id = [0u8; 32];
            id.copy_from_slice(&self.payload);
            Some(ParticleId(id))
        } else {
            None
        }
    }

    /// Content hash of this wavelet — used for deduplication
    pub fn content_hash(&self) -> [u8; 32] {
        blake3::hash(&self.payload).into()
    }
}

/// A Dimension is a named property of a Particle.
/// Think: column in a table, field in a document, attribute of a file.
pub type Dimension = (String, Wavelet);

/// Gravity defines how particles relate to each other.
/// Unlike hard links or directory entries, gravity bonds are typed and weighted.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct GravityBond {
    /// Target particle
    pub target: ParticleId,
    /// Bond type
    pub kind: GravityKind,
    /// Bond strength (0.0 – 1.0). Stronger = closer relationship.
    pub strength: f32,
    /// Optional label
    pub label: Option<String>,
}

/// Types of gravity bonds
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum GravityKind {
    /// Hierarchical containment (parent → child, like directories)
    Contains = 0,
    /// Code dependency (imports, uses)
    DependsOn = 1,
    /// Semantic similarity (AI-detected)
    RelatedTo = 2,
    /// Version chain (previous → next)
    VersionOf = 3,
    /// Reference (documentation → code, test → implementation)
    References = 4,
    /// Composition (module → submodules)
    ComposedOf = 5,
    /// Derived from / forked from
    DerivedFrom = 6,
    /// Generated / computed by (e.g. KV cache by model)
    ComputedBy = 7,
    /// Custom user-defined bond
    Custom = 8,
}

impl GravityKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Contains,
            1 => Self::DependsOn,
            2 => Self::RelatedTo,
            3 => Self::VersionOf,
            4 => Self::References,
            5 => Self::ComposedOf,
            6 => Self::DerivedFrom,
            7 => Self::ComputedBy,
            _ => Self::Custom,
        }
    }
}

/// A Particle is the atomic unit of DEFS.
/// It replaces the traditional inode with a semantically rich, multi-dimensional object.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct Particle {
    /// Unique content-addressable ID (blake3 hash of canonical encoding)
    pub id: ParticleId,
    /// Named dimensions (content, name, acl, embedding, etc.)
    pub dimensions: BTreeMap<String, Wavelet>,
    /// Gravity bonds (relationships to other particles)
    pub gravity: Vec<GravityBond>,
    /// Creation timestamp (nanoseconds since epoch)
    pub created_at_ns: u64,
    /// Last modification timestamp
    pub modified_at_ns: u64,
}

impl Particle {
    pub fn new(id: ParticleId) -> Self {
        Self {
            id,
            dimensions: BTreeMap::new(),
            gravity: Vec::new(),
            created_at_ns: 0,
            modified_at_ns: 0,
        }
    }

    /// Set a dimension value
    pub fn set_dimension(&mut self, name: &str, wavelet: Wavelet) {
        self.dimensions.insert(String::from(name), wavelet);
        self.modified_at_ns = 0; // caller should set real timestamp
    }

    /// Get a dimension value
    pub fn dimension(&self, name: &str) -> Option<&Wavelet> {
        self.dimensions.get(name)
    }

    /// Add a gravity bond
    pub fn add_bond(&mut self, target: ParticleId, kind: GravityKind, strength: f32) {
        self.gravity.push(GravityBond {
            target,
            kind,
            strength,
            label: None,
        });
    }

    /// Find bonds of a specific kind
    pub fn bonds_by_kind(&self, kind: GravityKind) -> Vec<&GravityBond> {
        self.gravity.iter().filter(|b| b.kind == kind).collect()
    }

    /// Get the content dimension (the "file bytes")
    pub fn content(&self) -> Option<&Wavelet> {
        self.dimensions.get("content")
    }

    /// Get the name dimension
    pub fn name(&self) -> Option<&str> {
        self.dimension("name").and_then(|w| w.as_str())
    }

    /// Compute canonical content hash for this particle
    /// Used for deduplication and integrity verification
    pub fn canonical_hash(&self) -> ParticleId {
        // Hash all dimension payloads in sorted order
        let mut hasher = blake3::Hasher::new();
        for (name, wavelet) in &self.dimensions {
            hasher.update(name.as_bytes());
            hasher.update(&wavelet.payload);
        }
        ParticleId(hasher.finalize().into())
    }
}

/// A Singularity is a dense cluster of particles where schema emerges.
/// It replaces the traditional directory — not a container, but an emergent grouping.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct Singularity {
    pub id: u64,
    /// Particles belonging to this singularity
    pub particles: Vec<ParticleId>,
    /// Signature: which dimensions are common across particles
    pub dimensional_signature: Vec<String>,
    /// Human-readable label (optional — can be auto-generated)
    pub label: Option<String>,
}

impl Singularity {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            particles: Vec::new(),
            dimensional_signature: Vec::new(),
            label: None,
        }
    }

    pub fn add_particle(&mut self, id: ParticleId) {
        if !self.particles.contains(&id) {
            self.particles.push(id);
        }
    }

    pub fn remove_particle(&mut self, id: &ParticleId) {
        self.particles.retain(|p| p != id);
    }
}

/// Resonance encodes what a particle IS — its type, format, encoding, and semantic properties.
/// This is a convenience builder for common metadata dimensions.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub struct Resonance {
    pub content_type: String,
    pub encoding: String,
    pub role: SemanticRole,
    pub tags: Vec<String>,
    pub language: Option<String>,
    pub quality: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
pub enum SemanticRole {
    Unknown,
    Source,
    Test,
    Config,
    Documentation,
    Asset,
    Data,
    Model,
    Cache,
}

impl Resonance {
    pub fn new(content_type: &str) -> Self {
        Self {
            content_type: String::from(content_type),
            encoding: String::from("utf8"),
            role: SemanticRole::Unknown,
            tags: Vec::new(),
            language: None,
            quality: 0.0,
        }
    }

    /// Apply resonance as dimensions on a particle
    pub fn apply_to(&self, particle: &mut Particle) {
        particle.set_dimension("content_type", Wavelet::from_string(&self.content_type));
        particle.set_dimension("encoding", Wavelet::from_string(&self.encoding));
        particle.set_dimension(
            "role",
            Wavelet::from_string(match self.role {
                SemanticRole::Unknown => "unknown",
                SemanticRole::Source => "source",
                SemanticRole::Test => "test",
                SemanticRole::Config => "config",
                SemanticRole::Documentation => "documentation",
                SemanticRole::Asset => "asset",
                SemanticRole::Data => "data",
                SemanticRole::Model => "model",
                SemanticRole::Cache => "cache",
            }),
        );
        if let Some(lang) = &self.language {
            particle.set_dimension("language", Wavelet::from_string(lang));
        }
        particle.set_dimension("quality", Wavelet::from_float64(self.quality as f64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_particle_id_from_content() {
        let data = b"hello world";
        let id1 = ParticleId::from_content(data);
        let id2 = ParticleId::from_content(data);
        assert_eq!(id1, id2, "Same content must produce same ID");
        assert!(!id1.is_null());
    }

    #[test]
    fn test_wavelet_string() {
        let w = Wavelet::from_string("hello");
        assert_eq!(w.tag, TypeTag::String);
        assert_eq!(w.as_str(), Some("hello"));
        assert_eq!(w.as_int64(), None);
    }

    #[test]
    fn test_wavelet_binary() {
        let data = vec![0u8, 1, 2, 3];
        let w = Wavelet::from_binary(&data);
        assert_eq!(w.tag, TypeTag::Binary);
        assert_eq!(w.as_binary(), Some(&data[..]));
    }

    #[test]
    fn test_wavelet_int64() {
        let w = Wavelet::from_int64(-42);
        assert_eq!(w.tag, TypeTag::Int64);
        assert_eq!(w.as_int64(), Some(-42));
    }

    #[test]
    fn test_wavelet_ref() {
        let target = ParticleId::from_content(b"target");
        let w = Wavelet::from_ref(&target);
        assert_eq!(w.tag, TypeTag::Ref);
        assert_eq!(w.as_particle_ref(), Some(target));
    }

    #[test]
    fn test_particle_dimensions() {
        let id = ParticleId::from_content(b"test");
        let mut p = Particle::new(id);
        p.set_dimension("name", Wavelet::from_string("report.docx"));
        p.set_dimension("content", Wavelet::from_binary(b"file bytes"));

        assert_eq!(p.name(), Some("report.docx"));
        assert!(p.content().is_some());
        assert_eq!(p.dimension("missing"), None);
    }

    #[test]
    fn test_gravity_bonds() {
        let id1 = ParticleId::from_content(b"p1");
        let id2 = ParticleId::from_content(b"p2");
        let mut p = Particle::new(id1);
        p.add_bond(id2, GravityKind::DependsOn, 0.95);

        let bonds = p.bonds_by_kind(GravityKind::DependsOn);
        assert_eq!(bonds.len(), 1);
        assert_eq!(bonds[0].target, id2);
        assert!((bonds[0].strength - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_singularity() {
        let mut s = Singularity::new(1);
        let pid = ParticleId::from_content(b"doc");
        s.add_particle(pid);
        s.add_particle(pid); // duplicate, should be ignored
        assert_eq!(s.particles.len(), 1);

        s.remove_particle(&pid);
        assert_eq!(s.particles.len(), 0);
    }

    #[test]
    fn test_resonance_apply() {
        let id = ParticleId::from_content(b"test");
        let mut p = Particle::new(id);
        let r = Resonance::new("application/pdf");
        r.apply_to(&mut p);

        assert_eq!(
            p.dimension("content_type").unwrap().as_str(),
            Some("application/pdf")
        );
        assert_eq!(p.dimension("role").unwrap().as_str(), Some("unknown"));
    }
}
