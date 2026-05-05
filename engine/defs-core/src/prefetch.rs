//! # Predictive Prefetch Engine
//!
//! Learns access patterns and prefetches predicted next particles.
//! Uses gravity bond strength to inform predictions.

use alloc::vec::Vec;

use crate::particle::{GravityKind, ParticleId};

/// A recorded access event
#[derive(Clone, Debug)]
pub struct AccessRecord {
    pub particle_id: ParticleId,
    pub timestamp_ns: u64,
    pub access_type: AccessType,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccessType {
    Read,
    Write,
    Open,
    Stat,
}

/// A detected access pattern (sequence → next particle)
#[derive(Clone, Debug)]
pub struct AccessPattern {
    pub sequence: Vec<ParticleId>,
    pub next_particle: ParticleId,
    pub frequency: u32,
    pub last_seen_ns: u64,
}

/// Predictive prefetch engine
pub struct PrefetchEngine {
    pub access_log: Vec<AccessRecord>,
    pub patterns: Vec<AccessPattern>,
    pub max_log_size: usize,
    pub prefetch_queue: Vec<ParticleId>,
    pub hits: u64,
    pub misses: u64,
    /// Gravity-aware: prefetch strongly bonded particles
    pub gravity_prefetch: bool,
}

impl PrefetchEngine {
    pub fn new(max_log: usize) -> Self {
        Self {
            access_log: Vec::new(),
            patterns: Vec::new(),
            max_log_size: max_log,
            prefetch_queue: Vec::new(),
            hits: 0,
            misses: 0,
            gravity_prefetch: true,
        }
    }

    pub fn record_access(&mut self, particle_id: ParticleId, timestamp_ns: u64, atype: AccessType) {
        self.access_log.push(AccessRecord {
            particle_id,
            timestamp_ns,
            access_type: atype,
        });

        if self.access_log.len() > self.max_log_size {
            self.access_log.remove(0);
        }

        if self.prefetch_queue.contains(&particle_id) {
            self.hits += 1;
            self.prefetch_queue.retain(|&id| id != particle_id);
        } else {
            self.misses += 1;
        }

        self.detect_patterns();
    }

    fn detect_patterns(&mut self) {
        if self.access_log.len() < 3 {
            return;
        }

        let len = self.access_log.len();
        let last2 = [
            self.access_log[len - 2].particle_id.clone(),
            self.access_log[len - 1].particle_id.clone(),
        ];

        // Look for: [A, B] → C patterns
        for i in 0..len - 2 {
            if self.access_log[i].particle_id == last2[0]
                && self.access_log[i + 1].particle_id == last2[1]
            {
                let next = self.access_log[i + 2].particle_id.clone();

                if let Some(pattern) = self
                    .patterns
                    .iter_mut()
                    .find(|p| p.sequence == last2 && p.next_particle == next)
                {
                    pattern.frequency += 1;
                    pattern.last_seen_ns = self.access_log[len - 1].timestamp_ns;
                } else {
                    self.patterns.push(AccessPattern {
                        sequence: last2.to_vec(),
                        next_particle: next,
                        frequency: 1,
                        last_seen_ns: self.access_log[len - 1].timestamp_ns,
                    });
                }
            }
        }
    }

    /// Predict next particles based on access patterns
    pub fn predict_next(&self) -> Vec<ParticleId> {
        let mut predictions = Vec::new();
        if self.access_log.len() < 2 {
            return predictions;
        }

        let len = self.access_log.len();
        let last2 = [
            self.access_log[len - 2].particle_id.clone(),
            self.access_log[len - 1].particle_id.clone(),
        ];

        for pattern in &self.patterns {
            if pattern.sequence == last2 && pattern.frequency > 1 {
                predictions.push(pattern.next_particle.clone());
            }
        }

        predictions
    }

    /// Gravity-aware prefetch: suggest strongly bonded particles
    pub fn predict_from_gravity<'a>(
        &self,
        _current_id: &ParticleId,
        bonds: &'a [(ParticleId, GravityKind, f32)],
    ) -> Vec<(ParticleId, f32)> {
        let mut predictions: Vec<(ParticleId, f32)> = bonds
            .iter()
            .map(|(id, _, strength)| (id.clone(), *strength))
            .filter(|(_, strength)| *strength > 0.5)
            .collect();

        predictions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal));
        predictions
    }

    pub fn hit_rate(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f32 / total as f32
        }
    }

    pub fn queue_prefetch(&mut self, particle_ids: Vec<ParticleId>) {
        for id in particle_ids {
            if !self.prefetch_queue.contains(&id) {
                self.prefetch_queue.push(id);
            }
        }
    }

    pub fn prefetch_queue_len(&self) -> usize {
        self.prefetch_queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::ParticleId;

    #[test]
    fn test_pattern_detection() {
        let mut engine = PrefetchEngine::new(100);
        let a = ParticleId::from_content(b"a");
        let b = ParticleId::from_content(b"b");
        let c = ParticleId::from_content(b"c");

        // Record pattern: a → b → c (multiple times)
        engine.record_access(a.clone(), 1, AccessType::Read);
        engine.record_access(b.clone(), 2, AccessType::Read);
        engine.record_access(c.clone(), 3, AccessType::Read);

        engine.record_access(a.clone(), 4, AccessType::Read);
        engine.record_access(b.clone(), 5, AccessType::Read);
        engine.record_access(c.clone(), 6, AccessType::Read);

        // Now access a, b → should predict c
        engine.record_access(a.clone(), 7, AccessType::Read);
        engine.record_access(b.clone(), 8, AccessType::Read);

        let predictions = engine.predict_next();
        assert_eq!(predictions.len(), 1);
        assert_eq!(predictions[0], c);
    }

    #[test]
    fn test_gravity_prefetch() {
        let engine = PrefetchEngine::new(100);
        let current = ParticleId::from_content(b"main");
        let bonds = vec![
            (
                ParticleId::from_content(b"config"),
                GravityKind::DependsOn,
                0.9,
            ),
            (
                ParticleId::from_content(b"utils"),
                GravityKind::DependsOn,
                0.3,
            ),
        ];

        let predictions = engine.predict_from_gravity(&current, &bonds);
        assert_eq!(predictions.len(), 1);
        assert_eq!(predictions[0].0, ParticleId::from_content(b"config"));
    }

    #[test]
    fn test_hit_rate() {
        let mut engine = PrefetchEngine::new(100);
        let a = ParticleId::from_content(b"a");

        engine.queue_prefetch(vec![a.clone()]);
        engine.record_access(a.clone(), 1, AccessType::Read);

        assert!(engine.hit_rate() > 0.0);
    }
}
