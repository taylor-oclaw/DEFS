//! # Copy-on-Write Snapshots
//!
//! Version management for particles. Every write creates a new version;
//! old versions are preserved until garbage collected.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::particle::{Particle, ParticleId};

/// A snapshot captures the state of a particle at a point in time
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    pub version: u64,
    pub timestamp_ns: u64,
    pub particle_id: ParticleId,
    pub description: String,
    /// Hash of the particle at this snapshot
    pub content_hash: [u8; 32],
}

/// Manages version chains for particles
pub struct SnapshotManager {
    /// particle_id → ordered list of snapshots (oldest first)
    chains: BTreeMap<ParticleId, Vec<Snapshot>>,
    /// Maximum snapshots to keep per particle (0 = unlimited)
    max_snapshots: usize,
    next_version: u64,
}

impl SnapshotManager {
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            chains: BTreeMap::new(),
            max_snapshots,
            next_version: 1,
        }
    }

    /// Create a snapshot of a particle before modification (CoW)
    pub fn snapshot_before_write(&mut self, particle: &Particle, description: &str) -> u64 {
        let version = self.next_version;
        self.next_version += 1;

        let snapshot = Snapshot {
            version,
            timestamp_ns: 0, // caller should set real time
            particle_id: particle.id.clone(),
            description: String::from(description),
            content_hash: particle.canonical_hash().0,
        };

        let chain = self
            .chains
            .entry(particle.id.clone())
            .or_insert_with(Vec::new);
        chain.push(snapshot);

        // Prune old snapshots if over limit
        if self.max_snapshots > 0 && chain.len() > self.max_snapshots {
            let to_remove = chain.len() - self.max_snapshots;
            chain.drain(0..to_remove);
        }

        version
    }

    /// Get all snapshots for a particle
    pub fn get_chain(&self, id: &ParticleId) -> Vec<&Snapshot> {
        self.chains
            .get(id)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    /// Get a specific snapshot version
    pub fn get_version(&self, id: &ParticleId, version: u64) -> Option<&Snapshot> {
        self.chains.get(id)?.iter().find(|s| s.version == version)
    }

    /// Get the latest snapshot for a particle
    pub fn latest(&self, id: &ParticleId) -> Option<&Snapshot> {
        self.chains.get(id)?.last()
    }

    /// Diff two versions (placeholder — would compare dimension hashes)
    pub fn diff(&self, id: &ParticleId, v1: u64, v2: u64) -> Option<SnapshotDiff> {
        let s1 = self.get_version(id, v1)?;
        let s2 = self.get_version(id, v2)?;
        Some(SnapshotDiff {
            from_version: v1,
            to_version: v2,
            hash_changed: s1.content_hash != s2.content_hash,
        })
    }

    /// Prune snapshots older than a given version
    pub fn prune_before_version(&mut self, id: &ParticleId, version: u64) {
        if let Some(chain) = self.chains.get_mut(id) {
            chain.retain(|s| s.version >= version);
        }
    }

    /// Total snapshots across all particles
    pub fn total_snapshots(&self) -> usize {
        self.chains.values().map(|v| v.len()).sum()
    }

    /// Particles with snapshots
    pub fn tracked_particles(&self) -> usize {
        self.chains.len()
    }
}

#[derive(Clone, Debug)]
pub struct SnapshotDiff {
    pub from_version: u64,
    pub to_version: u64,
    pub hash_changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::{ParticleId, Wavelet};

    #[test]
    fn test_snapshot_chain() {
        let mut sm = SnapshotManager::new(10);
        let id = ParticleId::from_content(b"doc");
        let mut p = Particle::new(id);
        p.set_dimension("content", Wavelet::from_binary(b"v1"));

        let v1 = sm.snapshot_before_write(&p, "initial");
        assert_eq!(v1, 1);

        p.set_dimension("content", Wavelet::from_binary(b"v2"));
        let v2 = sm.snapshot_before_write(&p, "edit 1");
        assert_eq!(v2, 2);

        let chain = sm.get_chain(&id);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].version, 1);
        assert_eq!(chain[1].version, 2);
    }

    #[test]
    fn test_snapshot_pruning() {
        let mut sm = SnapshotManager::new(3);
        let id = ParticleId::from_content(b"doc");

        for i in 0..5 {
            let mut p = Particle::new(id);
            p.set_dimension("v", Wavelet::from_int64(i));
            sm.snapshot_before_write(&p, &format!("v{}", i));
        }

        let chain = sm.get_chain(&id);
        assert_eq!(chain.len(), 3); // max 3 kept
        assert_eq!(chain[0].version, 3); // oldest pruned
    }

    #[test]
    fn test_snapshot_diff() {
        let mut sm = SnapshotManager::new(10);
        let id = ParticleId::from_content(b"doc");
        let mut p = Particle::new(id);
        p.set_dimension("content", Wavelet::from_binary(b"v1"));
        let v1 = sm.snapshot_before_write(&p, "");

        p.set_dimension("content", Wavelet::from_binary(b"v2"));
        let v2 = sm.snapshot_before_write(&p, "");

        let diff = sm.diff(&id, v1, v2).unwrap();
        assert!(diff.hash_changed);
    }
}
