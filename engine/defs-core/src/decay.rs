//! # Lifecycle Management (Decay)
//!
//! Automatic lifecycle policies for particles:
//! hot → warm → cold → archive → delete

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::particle::ParticleId;

/// Lifecycle temperature states
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Temperature {
    #[default]
    Cold, // Rarely accessed, can be compressed
    Hot,     // Frequently accessed, keep in fast storage
    Warm,    // Occasionally accessed
    Archive, // Almost never accessed, move to cold storage
    Delete,  // Marked for deletion
}

/// Action to take when a particle decays
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecayAction {
    Compress,
    MoveToCold,
    Archive,
    Delete,
    Notify,
}

/// A decay policy defines lifecycle rules
#[derive(Clone, Debug)]
pub struct DecayPolicy {
    pub name: String,
    pub max_age_seconds: u64,
    pub max_accesses_before_warm: u32,
    pub max_accesses_before_cold: u32,
    pub action: DecayAction,
    pub pattern: String, // regex-like pattern for matching particle names
    pub enabled: bool,
}

/// Tracks access statistics for a particle
#[derive(Clone, Debug, Default)]
pub struct AccessStats {
    pub last_accessed_ns: u64,
    pub access_count: u64,
    pub read_count: u64,
    pub write_count: u64,
    pub temperature: Temperature,
}

/// Decay engine manages particle lifecycles
pub struct DecayEngine {
    pub policies: Vec<DecayPolicy>,
    pub stats: BTreeMap<ParticleId, AccessStats>,
    pub decayed: Vec<(ParticleId, DecayAction)>,
}

impl DecayEngine {
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
            stats: BTreeMap::new(),
            decayed: Vec::new(),
        }
    }

    pub fn add_policy(&mut self, name: &str, max_age: u64, action: DecayAction, pattern: &str) {
        self.policies.push(DecayPolicy {
            name: String::from(name),
            max_age_seconds: max_age,
            max_accesses_before_warm: 10,
            max_accesses_before_cold: 100,
            action,
            pattern: String::from(pattern),
            enabled: true,
        });
    }

    pub fn record_access(&mut self, id: &ParticleId, timestamp_ns: u64, is_write: bool) {
        let stats = self
            .stats
            .entry(id.clone())
            .or_insert_with(AccessStats::default);
        stats.last_accessed_ns = timestamp_ns;
        stats.access_count += 1;
        if is_write {
            stats.write_count += 1;
        } else {
            stats.read_count += 1;
        }

        // Update temperature based on access count
        stats.temperature = if stats.access_count <= 5 {
            Temperature::Hot
        } else if stats.access_count <= 50 {
            Temperature::Warm
        } else {
            Temperature::Cold
        };
    }

    pub fn check_decay(&mut self, now_ns: u64) -> Vec<(ParticleId, DecayAction)> {
        let mut expired = Vec::new();
        let now_seconds = now_ns / 1_000_000_000;

        for (id, stats) in &self.stats {
            let age_seconds = now_seconds.saturating_sub(stats.last_accessed_ns / 1_000_000_000);

            for policy in &self.policies {
                if !policy.enabled {
                    continue;
                }
                if age_seconds > policy.max_age_seconds {
                    expired.push((id.clone(), policy.action));
                    break;
                }
            }
        }

        self.decayed.extend(expired.clone());
        expired
    }

    pub fn get_temperature(&self, id: &ParticleId) -> Temperature {
        self.stats
            .get(id)
            .map(|s| s.temperature)
            .unwrap_or(Temperature::Cold)
    }

    pub fn get_stats(&self, id: &ParticleId) -> Option<&AccessStats> {
        self.stats.get(id)
    }

    pub fn tracked_count(&self) -> usize {
        self.stats.len()
    }

    pub fn policy_count(&self) -> usize {
        self.policies.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::ParticleId;

    #[test]
    fn test_decay_policy() {
        let mut engine = DecayEngine::new();
        engine.add_policy("tmp", 3600, DecayAction::Delete, "*.tmp");

        let id = ParticleId::from_content(b"file");
        engine.record_access(&id, 0, false);
        engine.record_access(&id, 1_000_000_000, false);

        assert_eq!(engine.get_temperature(&id), Temperature::Hot);

        // Not expired yet
        let expired = engine.check_decay(3_000_000_000);
        assert!(expired.is_empty());

        // Expired (age > 3600 seconds)
        let expired = engine.check_decay(5_000_000_000_000);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].1, DecayAction::Delete);
    }

    #[test]
    fn test_temperature_progression() {
        let mut engine = DecayEngine::new();
        let id = ParticleId::from_content(b"file");

        for i in 0..5 {
            engine.record_access(&id, i * 1_000_000_000, false);
        }

        let stats = engine.get_stats(&id).unwrap();
        assert_eq!(stats.access_count, 5);
        assert!(stats.read_count > 0);
    }
}
