use alloc::string::String;
use alloc::vec::Vec;

pub type InodeNum = u64;
pub type BlockNum = u64;

#[derive(Clone, Copy, PartialEq)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    Device,
    Pipe,
    Socket,
}

pub struct Permissions {
    mode: u16,
}

impl Permissions {
    pub fn from_mode(mode: u16) -> Self { Self { mode } }
    pub fn to_mode(&self) -> u16 { self.mode }
    pub fn owner_read(&self) -> bool { self.mode & 0o400 != 0 }
    pub fn owner_write(&self) -> bool { self.mode & 0o200 != 0 }
    pub fn owner_exec(&self) -> bool { self.mode & 0o100 != 0 }
    pub fn group_read(&self) -> bool { self.mode & 0o040 != 0 }
    pub fn world_read(&self) -> bool { self.mode & 0o004 != 0 }
}

/// Contiguous block range on disk
pub struct Extent {
    pub start_block: BlockNum,
    pub block_count: u32,
    pub file_offset: u64,
}

/// AI-native semantic tag
pub struct SemanticTag {
    pub key: String,
    pub value: String,
    pub confidence: f32,
    pub auto_generated: bool,
}

/// DEFS inode — the heart of the filesystem
pub struct Inode {
    pub inode_num: InodeNum,
    pub file_type: FileType,
    pub permissions: Permissions,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub blocks_used: u64,
    pub extents: Vec<Extent>,
    pub created_at: u64,
    pub modified_at: u64,
    pub accessed_at: u64,
    pub link_count: u32,
    pub flags: u32,
    /// SHA-256 content hash for dedup + integrity
    pub content_hash: [u8; 32],
    /// AI-generated semantic tags (PATENTABLE: auto-tagging)
    pub tags: Vec<SemanticTag>,
    /// Version counter for CoW snapshots
    pub version: u64,
    /// Parent snapshot inode (0 = none)
    pub snapshot_parent: InodeNum,
    /// Content type detected by AI
    pub content_type: ContentType,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ContentType {
    Unknown,
    Text,
    Image,
    Audio,
    Video,
    Archive,
    Executable,
    AiModel,
    AiDataset,
    SourceCode,
    Document,
    Database,
}

/// PATENTABLE: File flags for AI-native features
pub const FLAG_IMMUTABLE: u32 = 0x01;
pub const FLAG_APPEND_ONLY: u32 = 0x02;
pub const FLAG_NO_DEDUP: u32 = 0x04;
pub const FLAG_AI_TAGGED: u32 = 0x08;
pub const FLAG_MODEL_FILE: u32 = 0x10;
pub const FLAG_LAYER_ADDRESSABLE: u32 = 0x20;
pub const FLAG_COW_SNAPSHOT: u32 = 0x40;
pub const FLAG_ENCRYPTED: u32 = 0x80;
pub const FLAG_COMPRESSED: u32 = 0x100;
pub const FLAG_DECAY_ENABLED: u32 = 0x200;

impl Inode {
    pub fn new_file(num: InodeNum) -> Self {
        Self {
            inode_num: num,
            file_type: FileType::Regular,
            permissions: Permissions::from_mode(0o644),
            uid: 0, gid: 0,
            size: 0, blocks_used: 0,
            extents: Vec::new(),
            created_at: 0, modified_at: 0, accessed_at: 0,
            link_count: 1, flags: 0,
            content_hash: [0u8; 32],
            tags: Vec::new(),
            version: 1,
            snapshot_parent: 0,
            content_type: ContentType::Unknown,
        }
    }

    pub fn new_dir(num: InodeNum) -> Self {
        let mut i = Self::new_file(num);
        i.file_type = FileType::Directory;
        i.permissions = Permissions::from_mode(0o755);
        i
    }

    pub fn new_model(num: InodeNum) -> Self {
        let mut i = Self::new_file(num);
        i.content_type = ContentType::AiModel;
        i.flags = FLAG_MODEL_FILE | FLAG_LAYER_ADDRESSABLE;
        i
    }

    pub fn add_extent(&mut self, start: BlockNum, count: u32, offset: u64) {
        self.extents.push(Extent { start_block: start, block_count: count, file_offset: offset });
        self.blocks_used += count as u64;
    }

    pub fn add_tag(&mut self, key: &str, value: &str, confidence: f32) {
        self.tags.push(SemanticTag {
            key: String::from(key),
            value: String::from(value),
            confidence,
            auto_generated: true,
        });
        self.flags |= FLAG_AI_TAGGED;
    }

    pub fn find_tag(&self, key: &str) -> Option<&str> {
        self.tags.iter().find(|t| t.key == key).map(|t| t.value.as_str())
    }

    pub fn tags_by_confidence(&self, min_confidence: f32) -> Vec<&SemanticTag> {
        self.tags.iter().filter(|t| t.confidence >= min_confidence).collect()
    }

    pub fn is_model(&self) -> bool { self.content_type == ContentType::AiModel }
    pub fn is_snapshot(&self) -> bool { self.flags & FLAG_COW_SNAPSHOT != 0 }
    pub fn is_encrypted(&self) -> bool { self.flags & FLAG_ENCRYPTED != 0 }

    /// PATENTABLE: Create CoW snapshot of this inode
    pub fn create_snapshot(&self, new_num: InodeNum) -> Self {
        let mut snap = Self::new_file(new_num);
        snap.file_type = self.file_type;
        snap.size = self.size;
        snap.extents = Vec::new(); // shares blocks via CoW
        snap.snapshot_parent = self.inode_num;
        snap.flags = FLAG_COW_SNAPSHOT;
        snap.content_hash = self.content_hash;
        snap.version = self.version + 1;
        snap
    }
}
