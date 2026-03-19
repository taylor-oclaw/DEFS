use alloc::string::String;
use alloc::vec::Vec;
use alloc::string::String;

pub type InodeNum = u64;
pub type BlockNum = u64;

pub struct SnapshotVersion {
    pub version: u64,
    pub timestamp: u64,
    pub inode: InodeNum,
    pub description: String,
    pub changed_blocks: Vec<BlockNum>,
}

pub struct SnapshotTree {
    pub versions: Vec<SnapshotVersion>,
    pub current_version: u64,
    pub max_versions: u64,
}

impl SnapshotTree {
    pub fn new(max: u64) -> Self {
        Self {
            versions: Vec::new(),
            current_version: 0,
            max_versions: max,
        }
    }

    pub fn create_snapshot(&mut self, inode: InodeNum, desc: &str, changed: Vec<BlockNum>, timestamp: u64) -> u64 {
        self.current_version += 1;
        self.versions.push(SnapshotVersion {
            version: self.current_version,
            timestamp,
            inode,
            description: String::from(desc),
            changed_blocks: changed,
        });
        if self.versions.len() as u64 > self.max_versions {
            self.versions.remove(0);
        }
        self.current_version
    }

    pub fn get_version(&self, ver: u64) -> Option<&SnapshotVersion> {
        self.versions.iter().find(|v| v.version == ver)
    }

    pub fn versions_for_inode(&self, inode: InodeNum) -> Vec<&SnapshotVersion> {
        self.versions.iter().filter(|v| v.inode == inode).collect()
    }

    pub fn latest(&self) -> Option<&SnapshotVersion> {
        self.versions.last()
    }

    pub fn rollback_to(&self, ver: u64) -> Option<Vec<BlockNum>> {
        self.get_version(ver).map(|v| v.changed_blocks.clone())
    }

    pub fn prune_before(&mut self, timestamp: u64) {
        self.versions.retain(|v| v.timestamp >= timestamp);
    }

    pub fn total_changed_blocks(&self) -> usize {
        self.versions.iter().map(|v| v.changed_blocks.len()).sum()
    }
}
