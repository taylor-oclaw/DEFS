use alloc::string::String;
use alloc::vec::Vec;
use alloc::string::String;

pub type InodeNum = u64;

pub struct AccessRecord {
    pub inode: InodeNum,
    pub timestamp: u64,
    pub access_type: AccessType,
}

pub enum AccessType {
    Read,
    Write,
    Open,
    Stat,
}

pub struct AccessPattern {
    pub sequence: Vec<InodeNum>,
    pub frequency: u32,
    pub last_seen: u64,
}

pub struct PrefetchEngine {
    pub access_log: Vec<AccessRecord>,
    pub patterns: Vec<AccessPattern>,
    pub max_log_size: usize,
    pub prefetch_queue: Vec<InodeNum>,
    pub hits: u64,
    pub misses: u64,
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
        }
    }

    pub fn record_access(&mut self, inode: InodeNum, timestamp: u64, atype: AccessType) {
        self.access_log.push(AccessRecord { inode, timestamp, access_type: atype });
        if self.access_log.len() > self.max_log_size {
            self.access_log.remove(0);
        }
        if self.prefetch_queue.contains(&inode) {
            self.hits += 1;
            self.prefetch_queue.retain(|&i| i != inode);
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
        let last3: Vec<InodeNum> = self.access_log[len - 3..].iter().map(|a| a.inode).collect();
        for p in &mut self.patterns {
            if p.sequence.len() >= 3 && p.sequence[p.sequence.len() - 3..] == last3[..] {
                p.frequency += 1;
                return;
            }
        }
        self.patterns.push(AccessPattern {
            sequence: last3,
            frequency: 1,
            last_seen: self.access_log.last().map(|a| a.timestamp).unwrap_or(0),
        });
    }

    pub fn predict_next(&self) -> Vec<InodeNum> {
        let mut predictions = Vec::new();
        if self.access_log.len() < 2 {
            return predictions;
        }
        let last = self.access_log.last().unwrap().inode;
        for p in &self.patterns {
            if p.frequency > 1 {
                for (i, &inode) in p.sequence.iter().enumerate() {
                    if inode == last && i + 1 < p.sequence.len() {
                        predictions.push(p.sequence[i + 1]);
                    }
                }
            }
        }
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
}
