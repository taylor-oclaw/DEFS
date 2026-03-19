use alloc::string::String;
use alloc::vec::Vec;
use alloc::string::String;

pub type InodeNum = u64;

pub enum DecayAction {
    Delete,
    Compress,
    Archive,
    Notify,
}

pub struct DecayPolicy {
    pub name: String,
    pub max_age_seconds: u64,
    pub action: DecayAction,
    pub pattern: String,
    pub enabled: bool,
}

pub struct DecayEntry {
    pub inode: InodeNum,
    pub last_accessed: u64,
    pub policy_name: String,
}

pub struct DecayEngine {
    pub policies: Vec<DecayPolicy>,
    pub tracked: Vec<DecayEntry>,
}

impl DecayEngine {
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
            tracked: Vec::new(),
        }
    }

    pub fn add_policy(&mut self, name: &str, max_age: u64, action: DecayAction, pattern: &str) {
        self.policies.push(DecayPolicy {
            name: String::from(name),
            max_age_seconds: max_age,
            action,
            pattern: String::from(pattern),
            enabled: true,
        });
    }

    pub fn track_file(&mut self, inode: InodeNum, last_access: u64, policy: &str) {
        self.tracked.push(DecayEntry {
            inode,
            last_accessed: last_access,
            policy_name: String::from(policy),
        });
    }

    pub fn check_expired(&self, now: u64) -> Vec<(InodeNum, &DecayAction)> {
        let mut expired = Vec::new();
        for entry in &self.tracked {
            if let Some(policy) = self.policies.iter().find(|p| p.name == entry.policy_name && p.enabled) {
                if now - entry.last_accessed > policy.max_age_seconds {
                    expired.push((entry.inode, &policy.action));
                }
            }
        }
        expired
    }

    pub fn remove_tracked(&mut self, inode: InodeNum) {
        self.tracked.retain(|e| e.inode != inode);
    }

    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }

    pub fn policy_count(&self) -> usize {
        self.policies.len()
    }
}
