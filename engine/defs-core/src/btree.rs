use alloc::boxed::Box;
use alloc::string::String;

pub type InodeNum = u64;

pub struct DirEntry {
    pub name: String,
    pub inode: InodeNum,
    pub entry_type: u8,
}

pub struct BTreeNode {
    pub keys: Vec<String>,
    pub values: Vec<InodeNum>,
    pub children: Vec<Box<BTreeNode>>,
    pub is_leaf: bool,
    pub max_keys: usize,
}

impl BTreeNode {
    pub fn new(max_keys: usize, is_leaf: bool) -> Self {
        Self {
            keys: Vec::new(),
            values: Vec::new(),
            children: Vec::new(),
            is_leaf,
            max_keys,
        }
    }

    pub fn search(&self, name: &str) -> Option<InodeNum> {
        let mut i = 0;
        while i < self.keys.len() {
            if name == self.keys[i] {
                return Some(self.values[i]);
            } else if name < self.keys[i].as_str() {
                break;
            }
            i += 1;
        }
        if self.is_leaf {
            None
        } else if i < self.children.len() {
            self.children[i].search(name)
        } else {
            None
        }
    }

    pub fn insert(&mut self, name: String, inode: InodeNum) {
        let mut i = 0;
        while i < self.keys.len() && self.keys[i] < name {
            i += 1;
        }
        if i < self.keys.len() && self.keys[i] == name {
            self.values[i] = inode;
            return;
        }
        if self.is_leaf {
            self.keys.insert(i, name);
            self.values.insert(i, inode);
        } else {
            if i < self.children.len() {
                self.children[i].insert(name, inode);
            }
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let mut i = 0;
        while i < self.keys.len() {
            if self.keys[i] == name {
                self.keys.remove(i);
                self.values.remove(i);
                return true;
            }
            i += 1;
        }
        if !self.is_leaf {
            for child in &mut self.children {
                if child.remove(name) {
                    return true;
                }
            }
        }
        false
    }

    pub fn list_all(&self) -> Vec<DirEntry> {
        let mut entries = Vec::new();
        for (i, key) in self.keys.iter().enumerate() {
            entries.push(DirEntry {
                name: key.clone(),
                inode: self.values[i],
                entry_type: 0,
            });
        }
        if !self.is_leaf {
            for child in &self.children {
                entries.extend(child.list_all());
            }
        }
        entries
    }

    pub fn count(&self) -> usize {
        let mut c = self.keys.len();
        for child in &self.children {
            c += child.count();
        }
        c
    }
}
