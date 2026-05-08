//! # In-Memory Particle Store
//!
//! A reference implementation of particle storage with gravity indexing.
//! This is the "source of truth" for how DEFS manages particles before
//! persistence is added.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::particle::{GravityBond, GravityKind, Particle, ParticleId, Singularity, Wavelet};

/// Core error type for storage operations
#[derive(Debug, Clone, PartialEq)]
pub enum StoreError {
    NotFound,
    AlreadyExists,
    InvalidDimension,
    Corrupted,
    IoError(String),
}

#[cfg(feature = "std")]
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound => write!(f, "Particle not found"),
            StoreError::AlreadyExists => write!(f, "Particle already exists"),
            StoreError::InvalidDimension => write!(f, "Invalid dimension"),
            StoreError::Corrupted => write!(f, "Store corrupted"),
            StoreError::IoError(s) => write!(f, "IO error: {}", s),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StoreError {}

/// Search query types for particle retrieval
#[derive(Debug, Clone, PartialEq)]
pub enum SearchQuery {
    /// Exact dimension match
    DimensionEquals { name: String, value: Wavelet },
    /// Dimension contains substring (for strings)
    DimensionContains { name: String, substring: String },
    /// Gravity bond traversal
    RelatedTo {
        id: ParticleId,
        kind: Option<GravityKind>,
        max_depth: u32,
    },
    /// Semantic similarity search by query string
    Semantic { query: String, k: usize },
    /// Find particles similar to a given particle
    SimilarTo { id: ParticleId, k: usize },
    /// Compound AND query
    And(Vec<SearchQuery>),
    /// Compound OR query
    Or(Vec<SearchQuery>),
}

/// In-memory particle store with gravity indexing
pub struct ParticleStore {
    particles: BTreeMap<ParticleId, Particle>,
    singularities: BTreeMap<u64, Singularity>,
    /// Gravity index: target_id → [source bonds] for fast incoming lookups
    gravity_incoming: BTreeMap<ParticleId, Vec<(ParticleId, usize)>>,
    next_singularity_id: u64,
}

impl ParticleStore {
    pub fn new() -> Self {
        Self {
            particles: BTreeMap::new(),
            singularities: BTreeMap::new(),
            gravity_incoming: BTreeMap::new(),
            next_singularity_id: 1,
        }
    }

    /// Store a particle. Replaces existing if same ID.
    pub fn write(&mut self, particle: Particle) -> Result<(), StoreError> {
        // Remove old gravity index entries for this particle
        self.gravity_incoming.retain(|_, refs| {
            refs.retain(|(pid, _)| pid != &particle.id);
            !refs.is_empty()
        });

        // Update gravity index
        for (idx, bond) in particle.gravity.iter().enumerate() {
            self.gravity_incoming
                .entry(bond.target.clone())
                .or_insert_with(Vec::new)
                .push((particle.id.clone(), idx));
        }

        self.particles.insert(particle.id.clone(), particle);
        Ok(())
    }

    /// Retrieve a particle by ID
    pub fn read(&self, id: &ParticleId) -> Result<Particle, StoreError> {
        self.particles.get(id).cloned().ok_or(StoreError::NotFound)
    }

    /// Delete a particle
    pub fn delete(&mut self, id: &ParticleId) -> Result<(), StoreError> {
        if let Some(particle) = self.particles.remove(id) {
            // Clean up gravity index
            for bond in &particle.gravity {
                if let Some(incoming) = self.gravity_incoming.get_mut(&bond.target) {
                    incoming.retain(|(src, _)| src != id);
                }
            }
            Ok(())
        } else {
            Err(StoreError::NotFound)
        }
    }

    /// Read a single dimension without loading the full particle
    pub fn read_dimension(
        &self,
        id: &ParticleId,
        name: &str,
    ) -> Result<Option<Wavelet>, StoreError> {
        let particle = self.particles.get(id).ok_or(StoreError::NotFound)?;
        Ok(particle.dimensions.get(name).cloned())
    }

    /// Write a single dimension
    pub fn write_dimension(
        &mut self,
        id: &ParticleId,
        name: &str,
        wavelet: Wavelet,
    ) -> Result<(), StoreError> {
        let particle = self.particles.get_mut(id).ok_or(StoreError::NotFound)?;
        particle.set_dimension(name, wavelet);
        Ok(())
    }

    /// Get outgoing gravity bonds
    pub fn outgoing_bonds(
        &self,
        id: &ParticleId,
        kind: Option<GravityKind>,
    ) -> Result<Vec<GravityBond>, StoreError> {
        let particle = self.particles.get(id).ok_or(StoreError::NotFound)?;
        Ok(match kind {
            Some(k) => particle
                .gravity
                .iter()
                .filter(|b| b.kind == k)
                .cloned()
                .collect(),
            None => particle.gravity.clone(),
        })
    }

    /// Get incoming gravity bonds (uses index)
    pub fn incoming_bonds(
        &self,
        id: &ParticleId,
        kind: Option<GravityKind>,
    ) -> Result<Vec<(ParticleId, GravityBond)>, StoreError> {
        let incoming = match self.gravity_incoming.get(id) {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };
        let mut result = Vec::new();
        for (src_id, bond_idx) in incoming {
            if let Some(src) = self.particles.get(src_id) {
                if let Some(bond) = src.gravity.get(*bond_idx) {
                    if kind.as_ref().map_or(true, |k| *k == bond.kind) {
                        result.push((src_id.clone(), bond.clone()));
                    }
                }
            }
        }
        Ok(result)
    }

    /// Search particles by query
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Particle>, StoreError> {
        match query {
            SearchQuery::DimensionEquals { name, value } => Ok(self
                .particles
                .values()
                .filter(|p| p.dimension(name) == Some(value))
                .cloned()
                .collect()),
            SearchQuery::DimensionContains { name, substring } => Ok(self
                .particles
                .values()
                .filter(|p| {
                    p.dimension(name)
                        .and_then(|w| w.as_str())
                        .map_or(false, |s| s.contains(substring))
                })
                .cloned()
                .collect()),
            SearchQuery::RelatedTo {
                id,
                kind,
                max_depth,
            } => {
                let mut results = Vec::new();
                let mut visited = Vec::new();
                self.traverse_graph(id, kind, *max_depth, 0, &mut visited, &mut results);
                Ok(results)
            }
            SearchQuery::Semantic { .. } => {
                // Placeholder: embedding index lives in PersistentStore
                Ok(Vec::new())
            }
            SearchQuery::SimilarTo { .. } => {
                // Placeholder: embedding index lives in PersistentStore
                Ok(Vec::new())
            }
            SearchQuery::And(queries) => {
                let mut results: Vec<Particle> = self.particles.values().cloned().collect();
                for q in queries {
                    let matched = self.search(q)?;
                    let matched_ids: Vec<_> = matched.iter().map(|p| p.id.clone()).collect();
                    results.retain(|p| matched_ids.contains(&p.id));
                }
                Ok(results)
            }
            SearchQuery::Or(queries) => {
                let mut seen = Vec::new();
                let mut results = Vec::new();
                for q in queries {
                    for p in self.search(q)? {
                        if !seen.contains(&p.id) {
                            seen.push(p.id.clone());
                            results.push(p);
                        }
                    }
                }
                Ok(results)
            }
        }
    }

    fn traverse_graph(
        &self,
        id: &ParticleId,
        kind: &Option<GravityKind>,
        max_depth: u32,
        current_depth: u32,
        visited: &mut Vec<ParticleId>,
        results: &mut Vec<Particle>,
    ) {
        if current_depth > max_depth || visited.contains(id) {
            return;
        }
        visited.push(id.clone());

        if let Ok(particle) = self.read(id) {
            for bond in &particle.gravity {
                if kind.as_ref().map_or(true, |k| *k == bond.kind) {
                    if let Ok(related) = self.read(&bond.target) {
                        results.push(related);
                        self.traverse_graph(
                            &bond.target,
                            kind,
                            max_depth,
                            current_depth + 1,
                            visited,
                            results,
                        );
                    }
                }
            }
        }
    }

    /// Create a new singularity
    pub fn create_singularity(&mut self, label: Option<String>) -> u64 {
        let id = self.next_singularity_id;
        self.next_singularity_id += 1;
        let mut s = Singularity::new(id);
        s.label = label;
        self.singularities.insert(id, s);
        id
    }

    /// Get a singularity
    pub fn singularity(&self, id: u64) -> Result<&Singularity, StoreError> {
        self.singularities.get(&id).ok_or(StoreError::NotFound)
    }

    /// Get mutable singularity
    pub fn singularity_mut(&mut self, id: u64) -> Result<&mut Singularity, StoreError> {
        self.singularities.get_mut(&id).ok_or(StoreError::NotFound)
    }

    /// List all particles
    pub fn all_particles(&self) -> Vec<&Particle> {
        self.particles.values().collect()
    }

    /// Count particles
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    /// Count singularities
    pub fn singularity_count(&self) -> usize {
        self.singularities.len()
    }
}

/// A stream of particles for scan operations
pub trait ParticleStream {
    fn next(&mut self) -> Option<Particle>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::{GravityKind, ParticleId, Wavelet};

    #[test]
    fn test_store_crud() {
        let mut store = ParticleStore::new();
        let id = ParticleId::from_content(b"hello");
        let mut p = crate::particle::Particle::new(id);
        p.set_dimension("name", Wavelet::from_string("test.txt"));
        p.set_dimension("content", Wavelet::from_binary(b"hello world"));

        store.write(p.clone()).unwrap();
        assert_eq!(store.particle_count(), 1);

        let retrieved = store.read(&id).unwrap();
        assert_eq!(retrieved.name(), Some("test.txt"));

        store.delete(&id).unwrap();
        assert_eq!(store.particle_count(), 0);
        assert!(store.read(&id).is_err());
    }

    #[test]
    fn test_dimension_access() {
        let mut store = ParticleStore::new();
        let id = ParticleId::from_content(b"doc");
        let mut p = crate::particle::Particle::new(id);
        p.set_dimension("content", Wavelet::from_binary(b"data"));
        store.write(p).unwrap();

        let dim = store.read_dimension(&id, "content").unwrap().unwrap();
        assert_eq!(dim.as_binary(), Some(&b"data"[..]));

        assert!(store.read_dimension(&id, "missing").unwrap().is_none());
    }

    #[test]
    fn test_gravity_index() {
        let mut store = ParticleStore::new();
        let id1 = ParticleId::from_content(b"p1");
        let id2 = ParticleId::from_content(b"p2");
        let id3 = ParticleId::from_content(b"p3");

        let mut p1 = crate::particle::Particle::new(id1);
        p1.add_bond(id2, GravityKind::Contains, 1.0);
        p1.add_bond(id3, GravityKind::RelatedTo, 0.8);
        store.write(p1).unwrap();

        // Outgoing from p1
        let outgoing = store.outgoing_bonds(&id1, None).unwrap();
        assert_eq!(outgoing.len(), 2);

        // Incoming to p2
        let incoming = store.incoming_bonds(&id2, None).unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].0, id1);

        // Filter by kind
        let related = store
            .outgoing_bonds(&id1, Some(GravityKind::RelatedTo))
            .unwrap();
        assert_eq!(related.len(), 1);
    }

    #[test]
    fn test_search_dimension_equals() {
        let mut store = ParticleStore::new();
        let id1 = ParticleId::from_content(b"a");
        let id2 = ParticleId::from_content(b"b");

        let mut p1 = crate::particle::Particle::new(id1);
        p1.set_dimension("type", Wavelet::from_string("pdf"));
        store.write(p1).unwrap();

        let mut p2 = crate::particle::Particle::new(id2);
        p2.set_dimension("type", Wavelet::from_string("docx"));
        store.write(p2).unwrap();

        let results = store
            .search(&SearchQuery::DimensionEquals {
                name: String::from("type"),
                value: Wavelet::from_string("pdf"),
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id1);
    }

    #[test]
    fn test_search_contains() {
        let mut store = ParticleStore::new();
        let id = ParticleId::from_content(b"doc");
        let mut p = crate::particle::Particle::new(id);
        p.set_dimension("name", Wavelet::from_string("quarterly_report.pdf"));
        store.write(p).unwrap();

        let results = store
            .search(&SearchQuery::DimensionContains {
                name: String::from("name"),
                substring: String::from("report"),
            })
            .unwrap();

        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_graph_traversal() {
        let mut store = ParticleStore::new();
        let id1 = ParticleId::from_content(b"root");
        let id2 = ParticleId::from_content(b"child1");
        let id3 = ParticleId::from_content(b"child2");

        // Write child particles first
        store.write(crate::particle::Particle::new(id2)).unwrap();
        store.write(crate::particle::Particle::new(id3)).unwrap();

        let mut root = crate::particle::Particle::new(id1);
        root.add_bond(id2, GravityKind::Contains, 1.0);
        root.add_bond(id3, GravityKind::Contains, 1.0);
        store.write(root).unwrap();

        let results = store
            .search(&SearchQuery::RelatedTo {
                id: id1,
                kind: Some(GravityKind::Contains),
                max_depth: 1,
            })
            .unwrap();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_singularity_management() {
        let mut store = ParticleStore::new();
        let sid = store.create_singularity(Some(String::from("Documents")));
        let pid = ParticleId::from_content(b"file");

        store.singularity_mut(sid).unwrap().add_particle(pid);
        assert_eq!(store.singularity(sid).unwrap().particles.len(), 1);
    }
}
