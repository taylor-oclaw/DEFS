//! # DEFS Virtual File System
//!
//! POSIX-like interface over the particle store.
//! Maps paths to particles, directories to `Contains` bonds,
//! and file content to the `content` dimension.

#[cfg(feature = "std")]
mod std_impl {
    use std::collections::{BTreeMap, HashMap};
    use std::path::Path;

    use crate::particle::{GravityKind, Particle, ParticleId, Wavelet};
    use crate::persist::PersistentStore;
    use crate::store::StoreError;
    use crate::volume::VolumeError;

    pub type InodeNum = u64;

    /// Persisted B-tree directory index.
    ///
    /// Stores a sorted map of entry names → child particle IDs as a binary
    /// dimension on the directory particle itself.  This gives O(log n)
    /// lookup/insert/delete instead of the O(n) linear scan over gravity
    /// bonds that we used in early phases.
    #[derive(Clone, Debug, Default)]
    pub struct DirIndex {
        entries: BTreeMap<String, ParticleId>,
    }

    impl DirIndex {
        pub fn new() -> Self {
            Self {
                entries: BTreeMap::new(),
            }
        }

        pub fn get(&self, name: &str) -> Option<&ParticleId> {
            self.entries.get(name)
        }

        pub fn insert(&mut self, name: String, id: ParticleId) {
            self.entries.insert(name, id);
        }

        pub fn remove(&mut self, name: &str) -> bool {
            self.entries.remove(name).is_some()
        }

        pub fn len(&self) -> usize {
            self.entries.len()
        }

        pub fn iter(&self) -> impl Iterator<Item = (&String, &ParticleId)> {
            self.entries.iter()
        }

        /// Serialize to a compact binary blob:
        ///   [count: u64]
        ///   for each entry: [name_len: u16] [name_bytes] [id: 32]
        pub fn serialize(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            let count = self.entries.len() as u64;
            buf.extend_from_slice(&count.to_le_bytes());
            for (name, id) in &self.entries {
                let name_bytes = name.as_bytes();
                buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(name_bytes);
                buf.extend_from_slice(&id.0);
            }
            buf
        }

        pub fn deserialize(data: &[u8]) -> Option<Self> {
            if data.len() < 8 {
                return None;
            }
            let count = u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            let mut offset = 8usize;
            let mut entries = BTreeMap::new();
            for _ in 0..count {
                if offset + 2 > data.len() {
                    break;
                }
                let name_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
                offset += 2;
                if offset + name_len + 32 > data.len() {
                    break;
                }
                let name = String::from_utf8_lossy(&data[offset..offset + name_len]).into_owned();
                offset += name_len;
                let mut id = [0u8; 32];
                id.copy_from_slice(&data[offset..offset + 32]);
                offset += 32;
                entries.insert(name, ParticleId(id));
            }
            Some(Self { entries })
        }
    }

    #[derive(Debug)]
    pub enum VfsError {
        NotFound,
        AlreadyExists,
        NotDirectory,
        IsDirectory,
        InvalidPath,
        Io(String),
    }

    impl std::fmt::Display for VfsError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                VfsError::NotFound => write!(f, "not found"),
                VfsError::AlreadyExists => write!(f, "already exists"),
                VfsError::NotDirectory => write!(f, "not a directory"),
                VfsError::IsDirectory => write!(f, "is a directory"),
                VfsError::InvalidPath => write!(f, "invalid path"),
                VfsError::Io(msg) => write!(f, "io error: {}", msg),
            }
        }
    }

    impl std::error::Error for VfsError {}

    impl From<StoreError> for VfsError {
        fn from(e: StoreError) -> Self {
            match e {
                StoreError::NotFound => VfsError::NotFound,
                _ => VfsError::Io(format!("{:?}", e)),
            }
        }
    }

    impl From<VolumeError> for VfsError {
        fn from(e: VolumeError) -> Self {
            VfsError::Io(format!("{:?}", e))
        }
    }

    #[derive(Clone, Debug)]
    pub struct DirEntry {
        pub name: String,
        pub inode: InodeNum,
        pub is_dir: bool,
    }

    /// File handle for open files
    pub struct FileHandle {
        pub inode: InodeNum,
        pub offset: u64,
        pub readable: bool,
        pub writable: bool,
    }

    /// Inode table: bidirectional mapping between inode numbers and particle IDs
    struct InodeTable {
        next: InodeNum,
        by_inode: HashMap<InodeNum, ParticleId>,
        by_particle: HashMap<ParticleId, InodeNum>,
    }

    impl InodeTable {
        fn new() -> Self {
            Self {
                next: 2, // 1 is reserved for root
                by_inode: HashMap::new(),
                by_particle: HashMap::new(),
            }
        }

        fn insert_root(&mut self, id: &ParticleId) {
            self.by_inode.insert(1, id.clone());
            self.by_particle.insert(id.clone(), 1);
        }

        fn get_or_insert(&mut self, id: &ParticleId) -> InodeNum {
            if let Some(&inode) = self.by_particle.get(id) {
                return inode;
            }
            let inode = self.next;
            self.next += 1;
            self.by_inode.insert(inode, id.clone());
            self.by_particle.insert(id.clone(), inode);
            inode
        }

        fn get_inode(&self, id: &ParticleId) -> Option<InodeNum> {
            self.by_particle.get(id).copied()
        }

        fn get_id(&self, inode: InodeNum) -> Option<&ParticleId> {
            self.by_inode.get(&inode)
        }

        fn remove(&mut self, id: &ParticleId) {
            if let Some(inode) = self.by_particle.remove(id) {
                self.by_inode.remove(&inode);
            }
        }
    }

    /// POSIX-like VFS over DEFS particles
    pub struct DefsVfs {
        store: PersistentStore,
        inodes: InodeTable,
        root_id: ParticleId,
        handles: HashMap<u64, FileHandle>,
        next_handle: u64,
        dir_index_cache: HashMap<ParticleId, DirIndex>,
    }

    impl DefsVfs {
        pub fn open(path: &Path) -> Result<Self, VfsError> {
            let mut store = if path.exists() {
                PersistentStore::open(path)?
            } else {
                PersistentStore::create(path, 5, "VFS")?
            };
            // NOTE: we do NOT call load_all() here anymore.
            // Particles are loaded on-demand via ensure_particle_loaded.

            // Find or create root particle
            let root_id = ParticleId::from_content(b"__vfs_root__");
            let root_exists = store.read(&root_id).is_ok() || store.load_particle(&root_id).is_ok();
            if !root_exists {
                let mut root = Particle::new(root_id.clone());
                root.set_dimension("name", Wavelet::from_string(""));
                root.set_dimension("__perm", Wavelet::from_int64(0o755));
                root.set_dimension("__uid", Wavelet::from_int64(1000));
                root.set_dimension("__gid", Wavelet::from_int64(1000));
                store.write(root)?;
                store.sync()?;
            }

            let mut inodes = InodeTable::new();
            inodes.insert_root(&root_id);

            Ok(Self {
                store,
                inodes,
                root_id,
                handles: HashMap::new(),
                next_handle: 1,
                dir_index_cache: HashMap::new(),
            })
        }

        pub fn sync(&mut self) -> Result<(), VfsError> {
            self.store.sync()?;
            Ok(())
        }

        pub fn store(&self) -> &PersistentStore {
            &self.store
        }

        pub fn store_mut(&mut self) -> &mut PersistentStore {
            &mut self.store
        }

        pub fn particle_count(&self) -> usize {
            self.store.particle_count()
        }

        pub fn info(&self) -> crate::volume::VolumeInfo {
            self.store.info()
        }

        /// Ensure a particle is loaded into memory (on-demand loading).
        fn ensure_particle_loaded(&mut self, id: &ParticleId) -> Result<(), VfsError> {
            if self.store.read(id).is_err() {
                self.store.load_particle(id)?;
            }
            self.inodes.get_or_insert(id);
            Ok(())
        }

        // --- Directory index helpers ---

        /// Load the directory index for a directory particle.
        /// Uses the cached copy if available; otherwise reads the `__dir_index`
        /// dimension or builds it from gravity bonds on first access.
        fn load_dir_index(&mut self, dir_id: &ParticleId) -> Result<DirIndex, VfsError> {
            if let Some(idx) = self.dir_index_cache.get(dir_id) {
                return Ok(idx.clone());
            }

            self.ensure_particle_loaded(dir_id)?;

            let mut built_from_bonds = false;
            let index = match self.store.read(dir_id) {
                Ok(particle) => match particle.dimension("__dir_index") {
                    Some(wavelet) => match wavelet.as_binary() {
                        Some(data) => DirIndex::deserialize(data).unwrap_or_else(|| {
                            built_from_bonds = true;
                            self.build_dir_index_from_bonds(dir_id).unwrap_or_default()
                        }),
                        None => {
                            built_from_bonds = true;
                            self.build_dir_index_from_bonds(dir_id)?
                        }
                    },
                    None => {
                        built_from_bonds = true;
                        self.build_dir_index_from_bonds(dir_id)?
                    }
                },
                Err(_) => DirIndex::new(),
            };

            if built_from_bonds && !dir_id.is_null() {
                let _ = self.save_dir_index(dir_id, &index);
            }

            self.dir_index_cache.insert(dir_id.clone(), index.clone());
            Ok(index)
        }

        /// Build a directory index by scanning the directory's Contains bonds.
        /// This is the fallback when no persisted index exists (backward compat).
        fn build_dir_index_from_bonds(
            &mut self,
            dir_id: &ParticleId,
        ) -> Result<DirIndex, VfsError> {
            let mut index = DirIndex::new();
            let particle = self.store.read(dir_id)?;
            for bond in particle.bonds_by_kind(GravityKind::Contains) {
                let _ = self.ensure_particle_loaded(&bond.target);
                if let Ok(child) = self.store.read(&bond.target) {
                    if let Some(name) = child.name() {
                        index.insert(name.to_string(), bond.target.clone());
                    }
                }
            }
            Ok(index)
        }

        /// Persist a directory index back to the directory particle.
        fn save_dir_index(
            &mut self,
            dir_id: &ParticleId,
            index: &DirIndex,
        ) -> Result<(), VfsError> {
            let mut particle = self.store.read(dir_id)?.clone();
            particle.set_dimension("__dir_index", Wavelet::from_binary(&index.serialize()));
            self.store.write(particle)?;
            self.dir_index_cache.insert(dir_id.clone(), index.clone());
            Ok(())
        }

        // --- Path resolution ---

        pub fn resolve_path(&mut self, path: &str) -> Result<(ParticleId, String), VfsError> {
            let path = path.trim_start_matches('/');
            if path.is_empty() || path == "." {
                return Ok((self.root_id.clone(), String::new()));
            }

            let components: Vec<&str> = path.split('/').collect();
            let mut current_id = self.root_id.clone();

            for (i, &name) in components.iter().enumerate() {
                if name.is_empty() || name == "." {
                    continue;
                }
                if name == ".." {
                    // Parent navigation: not fully supported without back-pointers
                    continue;
                }

                let index = self.load_dir_index(&current_id)?;
                if let Some(id) = index.get(name).cloned() {
                    current_id = id;
                } else if i == components.len() - 1 {
                    // Last component not found — return parent + name for creation
                    return Ok((current_id, name.to_string()));
                } else {
                    return Err(VfsError::NotFound);
                }
            }

            Ok((current_id, String::new()))
        }

        /// Look up a particle by path
        pub fn lookup(&mut self, path: &str) -> Result<(InodeNum, Particle), VfsError> {
            let (id, trailing) = self.resolve_path(path)?;
            if !trailing.is_empty() {
                return Err(VfsError::NotFound);
            }
            self.ensure_particle_loaded(&id)?;
            let particle = self.store.read(&id)?;
            let inode = self.inodes.get_inode(&id).unwrap_or(1);
            Ok((inode, particle))
        }

        /// Look up a child by name within a parent directory.
        pub fn lookup_by_name(
            &mut self,
            parent_inode: InodeNum,
            name: &str,
        ) -> Result<(InodeNum, Particle), VfsError> {
            let parent_id = self
                .inodes
                .get_id(parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let index = self.load_dir_index(&parent_id)?;
            let child_id = index.get(name).cloned().ok_or(VfsError::NotFound)?;
            self.ensure_particle_loaded(&child_id)?;
            let particle = self.store.read(&child_id)?;
            let inode = self.inodes.get_inode(&child_id).unwrap_or(0);
            Ok((inode, particle))
        }

        // --- Directory operations ---

        pub fn readdir(&mut self, inode: InodeNum) -> Result<Vec<DirEntry>, VfsError> {
            let id = self.inodes.get_id(inode).ok_or(VfsError::NotFound)?.clone();
            let mut entries = vec![
                DirEntry {
                    name: ".".into(),
                    inode,
                    is_dir: true,
                },
                DirEntry {
                    name: "..".into(),
                    inode: 1,
                    is_dir: true,
                },
            ];

            let index = self.load_dir_index(&id)?;
            for (name, child_id) in index.iter() {
                let _ = self.ensure_particle_loaded(child_id);
                let is_dir = if let Ok(child) = self.store.read(child_id) {
                    !child.bonds_by_kind(GravityKind::Contains).is_empty()
                } else {
                    false
                };
                let child_inode = self.inodes.get_inode(child_id).unwrap_or(0);
                entries.push(DirEntry {
                    name: name.clone(),
                    inode: child_inode,
                    is_dir,
                });
            }

            Ok(entries)
        }

        pub fn mkdir(&mut self, parent_inode: InodeNum, name: &str) -> Result<InodeNum, VfsError> {
            if name.is_empty() || name.contains('/') {
                return Err(VfsError::InvalidPath);
            }

            let parent_id = self
                .inodes
                .get_id(parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let mut index = self.load_dir_index(&parent_id)?;

            // Check for duplicate
            if index.get(name).is_some() {
                return Err(VfsError::AlreadyExists);
            }

            let id =
                ParticleId::from_content(format!("dir:{}/{}", parent_id.to_hex(), name).as_bytes());
            let mut dir = Particle::new(id.clone());
            dir.set_dimension("name", Wavelet::from_string(name));
            dir.set_dimension("__perm", Wavelet::from_int64(0o755));
            dir.set_dimension("__uid", Wavelet::from_int64(1000));
            dir.set_dimension("__gid", Wavelet::from_int64(1000));
            self.store.write(dir)?;

            let mut parent = self.store.read(&parent_id)?;
            parent.add_bond(id.clone(), GravityKind::Contains, 1.0);
            self.store.write(parent)?;

            index.insert(name.to_string(), id.clone());
            self.save_dir_index(&parent_id, &index)?;

            let inode = self.inodes.get_or_insert(&id);
            Ok(inode)
        }

        // --- File operations ---

        pub fn create(&mut self, parent_inode: InodeNum, name: &str) -> Result<InodeNum, VfsError> {
            if name.is_empty() || name.contains('/') {
                return Err(VfsError::InvalidPath);
            }

            let parent_id = self
                .inodes
                .get_id(parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let mut index = self.load_dir_index(&parent_id)?;

            // Check for duplicate
            if index.get(name).is_some() {
                return Err(VfsError::AlreadyExists);
            }

            let id = ParticleId::from_content(
                format!("file:{}/{}", parent_id.to_hex(), name).as_bytes(),
            );
            let mut file = Particle::new(id.clone());
            file.set_dimension("name", Wavelet::from_string(name));
            file.set_dimension(
                "content_type",
                Wavelet::from_string("application/octet-stream"),
            );
            file.set_dimension("content", Wavelet::from_binary(&[]));
            file.set_dimension("__perm", Wavelet::from_int64(0o644));
            file.set_dimension("__uid", Wavelet::from_int64(1000));
            file.set_dimension("__gid", Wavelet::from_int64(1000));
            self.store.write(file)?;

            let mut parent = self.store.read(&parent_id)?;
            parent.add_bond(id.clone(), GravityKind::Contains, 1.0);
            self.store.write(parent)?;

            index.insert(name.to_string(), id.clone());
            self.save_dir_index(&parent_id, &index)?;

            let inode = self.inodes.get_or_insert(&id);
            Ok(inode)
        }

        pub fn open_handle(
            &mut self,
            inode: InodeNum,
            read: bool,
            write: bool,
        ) -> Result<u64, VfsError> {
            let id = self.inodes.get_id(inode).ok_or(VfsError::NotFound)?;
            let _ = self.store.read(id)?; // validate exists
            let handle = self.next_handle;
            self.next_handle += 1;
            self.handles.insert(
                handle,
                FileHandle {
                    inode,
                    offset: 0,
                    readable: read,
                    writable: write,
                },
            );
            Ok(handle)
        }

        pub fn close_handle(&mut self, handle: u64) -> Result<(), VfsError> {
            self.handles.remove(&handle).ok_or(VfsError::NotFound)?;
            Ok(())
        }

        pub fn read(&mut self, handle: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
            let fh = self.handles.get_mut(&handle).ok_or(VfsError::NotFound)?;
            if !fh.readable {
                return Err(VfsError::Io("not readable".into()));
            }
            let id = self.inodes.get_id(fh.inode).ok_or(VfsError::NotFound)?;
            let particle = self.store.read(id)?;
            let content = particle
                .content()
                .map(|w| w.payload.clone())
                .unwrap_or_default();
            let offset = fh.offset as usize;
            let len = buf.len().min(content.len().saturating_sub(offset));
            if len == 0 {
                return Ok(0);
            }
            buf[..len].copy_from_slice(&content[offset..offset + len]);
            fh.offset += len as u64;
            Ok(len)
        }

        pub fn write(&mut self, handle: u64, buf: &[u8]) -> Result<usize, VfsError> {
            let fh = self.handles.get_mut(&handle).ok_or(VfsError::NotFound)?;
            if !fh.writable {
                return Err(VfsError::Io("not writable".into()));
            }
            let id = self
                .inodes
                .get_id(fh.inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let mut particle = self.store.read(&id)?;
            let mut content = particle
                .content()
                .map(|w| w.payload.clone())
                .unwrap_or_default();
            let offset = fh.offset as usize;
            if offset + buf.len() > content.len() {
                content.resize(offset + buf.len(), 0);
            }
            content[offset..offset + buf.len()].copy_from_slice(buf);
            particle.set_dimension("content", Wavelet::from_binary(&content));
            self.store.write(particle)?;
            fh.offset += buf.len() as u64;
            Ok(buf.len())
        }

        pub fn seek(&mut self, handle: u64, offset: u64) -> Result<(), VfsError> {
            let fh = self.handles.get_mut(&handle).ok_or(VfsError::NotFound)?;
            fh.offset = offset;
            Ok(())
        }

        pub fn truncate(&mut self, inode: InodeNum, size: u64) -> Result<(), VfsError> {
            let id = self.inodes.get_id(inode).ok_or(VfsError::NotFound)?.clone();
            let mut particle = self.store.read(&id)?;
            let mut content = particle
                .content()
                .map(|w| w.payload.clone())
                .unwrap_or_default();
            content.resize(size as usize, 0);
            particle.set_dimension("content", Wavelet::from_binary(&content));
            self.store.write(particle)?;
            Ok(())
        }

        pub fn unlink(&mut self, parent_inode: InodeNum, name: &str) -> Result<(), VfsError> {
            let parent_id = self
                .inodes
                .get_id(parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let mut index = self.load_dir_index(&parent_id)?;

            let target_id = index.get(name).cloned().ok_or(VfsError::NotFound)?;

            let mut parent = self.store.read(&parent_id)?;
            parent.gravity.retain(|b| b.target != target_id);
            self.store.write(parent)?;

            index.remove(name);
            self.save_dir_index(&parent_id, &index)?;

            // Delete the target particle (DEFS v1.0 has no hard links)
            self.store
                .delete(&target_id)
                .map_err(|e| VfsError::Io(format!("Delete failed: {:?}", e)))?;
            self.inodes.remove(&target_id);
            self.dir_index_cache.remove(&target_id);

            Ok(())
        }

        pub fn rename(
            &mut self,
            parent_inode: InodeNum,
            name: &str,
            new_name: &str,
        ) -> Result<(), VfsError> {
            let parent_id = self
                .inodes
                .get_id(parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let mut index = self.load_dir_index(&parent_id)?;

            let target_id = index.get(name).cloned().ok_or(VfsError::NotFound)?;

            let mut particle = self.store.read(&target_id)?;
            particle.set_dimension("name", Wavelet::from_string(new_name));
            self.store.write(particle)?;

            index.remove(name);
            index.insert(new_name.to_string(), target_id);
            self.save_dir_index(&parent_id, &index)?;
            Ok(())
        }

        /// Remove an empty directory.
        pub fn rmdir(&mut self, parent_inode: InodeNum, name: &str) -> Result<(), VfsError> {
            let parent_id = self
                .inodes
                .get_id(parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let index = self.load_dir_index(&parent_id)?;

            let target_id = index.get(name).cloned().ok_or(VfsError::NotFound)?;

            // Check if target is a directory
            let target = self.store.read(&target_id)?;
            if !target.bonds_by_kind(GravityKind::Contains).is_empty() {
                return Err(VfsError::NotDirectory);
            }

            // Use unlink which deletes the particle
            self.unlink(parent_inode, name)
        }

        /// Move an entry from one directory to another, optionally renaming it.
        pub fn rename_cross(
            &mut self,
            old_parent_inode: InodeNum,
            old_name: &str,
            new_parent_inode: InodeNum,
            new_name: &str,
        ) -> Result<(), VfsError> {
            let old_parent_id = self
                .inodes
                .get_id(old_parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();
            let new_parent_id = self
                .inodes
                .get_id(new_parent_inode)
                .ok_or(VfsError::NotFound)?
                .clone();

            let mut old_index = self.load_dir_index(&old_parent_id)?;
            let target_id = old_index.get(old_name).cloned().ok_or(VfsError::NotFound)?;

            // Remove from old parent
            let mut old_parent = self.store.read(&old_parent_id)?;
            old_parent.gravity.retain(|b| b.target != target_id);
            self.store.write(old_parent)?;
            old_index.remove(old_name);
            self.save_dir_index(&old_parent_id, &old_index)?;

            // Update target's name if changed
            if old_name != new_name {
                let mut particle = self.store.read(&target_id)?;
                particle.set_dimension("name", Wavelet::from_string(new_name));
                self.store.write(particle)?;
            }

            // Add to new parent
            let mut new_parent = self.store.read(&new_parent_id)?;
            new_parent.add_bond(target_id.clone(), GravityKind::Contains, 1.0);
            self.store.write(new_parent)?;

            let mut new_index = self.load_dir_index(&new_parent_id)?;
            new_index.insert(new_name.to_string(), target_id);
            self.save_dir_index(&new_parent_id, &new_index)?;

            Ok(())
        }

        pub fn is_dir(&self, inode: InodeNum) -> Result<bool, VfsError> {
            let id = self.inodes.get_id(inode).ok_or(VfsError::NotFound)?;
            let particle = self.store.read(id)?;
            Ok(!particle.bonds_by_kind(GravityKind::Contains).is_empty())
        }

        pub fn setattr(
            &mut self,
            inode: InodeNum,
            perm: Option<u16>,
            uid: Option<u32>,
            gid: Option<u32>,
        ) -> Result<(), VfsError> {
            let id = self.inodes.get_id(inode).ok_or(VfsError::NotFound)?.clone();
            let mut particle = self.store.read(&id)?;
            if let Some(p) = perm {
                particle.set_dimension("__perm", Wavelet::from_int64(p as i64));
            }
            if let Some(u) = uid {
                particle.set_dimension("__uid", Wavelet::from_int64(u as i64));
            }
            if let Some(g) = gid {
                particle.set_dimension("__gid", Wavelet::from_int64(g as i64));
            }
            self.store.write(particle)?;
            Ok(())
        }

        pub fn getattr(&self, inode: InodeNum) -> Result<VfsAttr, VfsError> {
            let id = self.inodes.get_id(inode).ok_or(VfsError::NotFound)?;
            let particle = self.store.read(id)?;
            let content_len = particle
                .content()
                .map(|w| w.payload.len() as u64)
                .unwrap_or(0);
            let is_dir = !particle.bonds_by_kind(GravityKind::Contains).is_empty();

            let default_perm = if is_dir { 0o755 } else { 0o644 };
            let perm = particle
                .dimension("__perm")
                .and_then(|w| w.as_int64())
                .map(|v| v as u16)
                .unwrap_or(default_perm);
            let uid = particle
                .dimension("__uid")
                .and_then(|w| w.as_int64())
                .map(|v| v as u32)
                .unwrap_or(1000);
            let gid = particle
                .dimension("__gid")
                .and_then(|w| w.as_int64())
                .map(|v| v as u32)
                .unwrap_or(1000);

            Ok(VfsAttr {
                inode,
                size: content_len,
                is_dir,
                created_ns: particle.created_at_ns,
                modified_ns: particle.modified_at_ns,
                perm,
                uid,
                gid,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub struct VfsAttr {
        pub inode: InodeNum,
        pub size: u64,
        pub is_dir: bool,
        pub created_ns: u64,
        pub modified_ns: u64,
        pub perm: u16,
        pub uid: u32,
        pub gid: u32,
    }

    // ------------------------------------------------------------------
    // Filesystem trait implementation (backend.rs)
    // ------------------------------------------------------------------
    use crate::backend::{DirEntry as BackendDirEntry, FileStat, Filesystem, FsError};

    impl From<VfsError> for FsError {
        fn from(e: VfsError) -> Self {
            match e {
                VfsError::NotFound => FsError::NotFound,
                VfsError::AlreadyExists => FsError::Exists,
                VfsError::NotDirectory => FsError::NotDirectory,
                VfsError::IsDirectory => FsError::IsDirectory,
                VfsError::InvalidPath => FsError::IoError,
                VfsError::Io(_) => FsError::IoError,
            }
        }
    }

    impl Filesystem for DefsVfs {
        fn name(&self) -> &str {
            "defs-vfs"
        }

        fn open(&mut self, path: &str, _flags: u32) -> Result<u64, FsError> {
            let (inode, _) = self.lookup(path)?;
            self.open_handle(inode, true, true).map_err(FsError::from)
        }

        fn read(&mut self, fd: u64, buf: &mut [u8], offset: u64) -> Result<usize, FsError> {
            self.seek(fd, offset).map_err(FsError::from)?;
            DefsVfs::read(self, fd, buf).map_err(FsError::from)
        }

        fn write(&mut self, fd: u64, buf: &[u8], offset: u64) -> Result<usize, FsError> {
            self.seek(fd, offset).map_err(FsError::from)?;
            DefsVfs::write(self, fd, buf).map_err(FsError::from)
        }

        fn close(&mut self, fd: u64) -> Result<(), FsError> {
            self.close_handle(fd).map_err(FsError::from)
        }

        fn mkdir(&mut self, path: &str, _mode: u16) -> Result<(), FsError> {
            let path = path.trim_end_matches('/');
            let (parent_id, name) = self.resolve_path(path)?;
            if name.is_empty() {
                return Err(FsError::Exists);
            }
            let parent_inode = self.inodes.get_inode(&parent_id).ok_or(FsError::NotFound)?;
            DefsVfs::mkdir(self, parent_inode, &name).map_err(FsError::from)?;
            DefsVfs::sync(self).map_err(FsError::from)?;
            Ok(())
        }

        fn readdir(&mut self, path: &str) -> Result<Vec<BackendDirEntry>, FsError> {
            let (inode, _) = self.lookup(path)?;
            let entries = DefsVfs::readdir(self, inode)?;
            Ok(entries
                .into_iter()
                .map(|e| BackendDirEntry {
                    name: e.name,
                    inode: e.inode,
                    entry_type: if e.is_dir { 4 } else { 8 },
                })
                .collect())
        }

        fn stat(&mut self, path: &str) -> Result<FileStat, FsError> {
            let (inode, _) = self.lookup(path)?;
            let attr = self.getattr(inode)?;
            let mode = if attr.is_dir {
                attr.perm | 0o40000
            } else {
                attr.perm | 0o100000
            };
            Ok(FileStat {
                ino: attr.inode,
                size: attr.size,
                blocks: (attr.size + 511) / 512,
                mode,
                uid: attr.uid,
                gid: attr.gid,
                atime: attr.modified_ns,
                mtime: attr.modified_ns,
                ctime: attr.created_ns,
            })
        }

        fn unlink(&mut self, path: &str) -> Result<(), FsError> {
            let path = path.trim_end_matches('/');
            let (parent_id, name) = self.resolve_path(path)?;
            if name.is_empty() {
                return Err(FsError::IsDirectory);
            }
            let parent_inode = self.inodes.get_inode(&parent_id).ok_or(FsError::NotFound)?;
            DefsVfs::unlink(self, parent_inode, &name).map_err(FsError::from)?;
            DefsVfs::sync(self).map_err(FsError::from)?;
            Ok(())
        }

        fn rename(&mut self, from: &str, to: &str) -> Result<(), FsError> {
            let from = from.trim_end_matches('/');
            let to = to.trim_end_matches('/');
            let (parent_id, name) = self.resolve_path(from)?;
            if name.is_empty() {
                return Err(FsError::NotFound);
            }
            let parent_inode = self.inodes.get_inode(&parent_id).ok_or(FsError::NotFound)?;

            // Extract new name from 'to' path
            let to_name = to.rsplit_once('/').map(|(_, n)| n).unwrap_or(to);
            DefsVfs::rename(self, parent_inode, &name, to_name).map_err(FsError::from)?;
            DefsVfs::sync(self).map_err(FsError::from)?;
            Ok(())
        }

        fn read_particle(&self, id: &ParticleId) -> Result<Particle, FsError> {
            self.store.read(id).map_err(|e| match e {
                StoreError::NotFound => FsError::NotFound,
                _ => FsError::IoError,
            })
        }

        fn write_particle(&mut self, particle: &Particle) -> Result<(), FsError> {
            self.store.write(particle.clone()).map_err(|e| match e {
                StoreError::NotFound => FsError::NotFound,
                _ => FsError::IoError,
            })?;
            DefsVfs::sync(self).map_err(|_| FsError::IoError)?;
            Ok(())
        }

        fn find_by_intent(&self, intent: &str) -> Result<Vec<ParticleId>, FsError> {
            let particles = self.store.all_particles();
            let ids: Vec<ParticleId> = particles
                .into_iter()
                .filter(|p| p.dimension("intent").and_then(|w| w.as_str()) == Some(intent))
                .map(|p| p.id.clone())
                .collect();
            Ok(ids)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        fn tmp_path(name: &str) -> PathBuf {
            PathBuf::from(format!("/tmp/test_vfs_{}.defs", name))
        }

        fn clean(name: &str) {
            let _ = std::fs::remove_file(tmp_path(name));
        }

        #[test]
        fn test_vfs_create_and_lookup() {
            let name = "create_lookup";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let file_inode = vfs.create(1, "hello.txt").unwrap();
            println!("Particles after create: {}", vfs.particle_count());
            vfs.sync().unwrap();

            let (inode, particle) = vfs.lookup("/hello.txt").unwrap();
            assert_eq!(inode, file_inode);
            assert_eq!(particle.name(), Some("hello.txt"));

            clean(name);
        }

        #[test]
        fn test_vfs_mkdir_and_readdir() {
            let name = "mkdir_readdir";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let dir_inode = vfs.mkdir(1, "docs").unwrap();
            vfs.create(dir_inode, "readme.md").unwrap();
            vfs.sync().unwrap();

            let entries = vfs.readdir(dir_inode).unwrap();
            let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
            assert!(names.contains(&"."));
            assert!(names.contains(&".."));
            assert!(names.contains(&"readme.md"));

            clean(name);
        }

        #[test]
        fn test_vfs_read_write() {
            let name = "read_write";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let file_inode = vfs.create(1, "data.bin").unwrap();
            let handle = vfs.open_handle(file_inode, true, true).unwrap();

            vfs.write(handle, b"hello world").unwrap();
            vfs.seek(handle, 0).unwrap();

            let mut buf = vec![0u8; 64];
            let n = vfs.read(handle, &mut buf).unwrap();
            assert_eq!(&buf[..n], b"hello world");

            vfs.close_handle(handle).unwrap();
            vfs.sync().unwrap();

            clean(name);
        }

        #[test]
        fn test_vfs_unlink() {
            let name = "unlink";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            vfs.create(1, "tmp.txt").unwrap();
            vfs.sync().unwrap();

            assert!(vfs.lookup("/tmp.txt").is_ok());
            vfs.unlink(1, "tmp.txt").unwrap();
            assert!(vfs.lookup("/tmp.txt").is_err());

            clean(name);
        }

        #[test]
        fn test_vfs_nested_paths() {
            let name = "nested";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let dir = vfs.mkdir(1, "a").unwrap();
            let subdir = vfs.mkdir(dir, "b").unwrap();
            vfs.create(subdir, "c.txt").unwrap();
            vfs.sync().unwrap();

            let (inode, _) = vfs.lookup("/a/b/c.txt").unwrap();
            assert!(inode > 1);

            clean(name);
        }

        #[test]
        fn test_vfs_dir_index_persisted() {
            let name = "dir_index_persist";
            clean(name);

            // Phase 1: create files and sync
            {
                let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
                for i in 0..10 {
                    vfs.create(1, &format!("file{:02}.txt", i)).unwrap();
                }
                vfs.sync().unwrap();

                // Verify all lookups work before close
                for i in 0..10 {
                    let path = format!("/file{:02}.txt", i);
                    assert!(
                        vfs.lookup(&path).is_ok(),
                        "pre-close lookup failed for {}",
                        path
                    );
                }

                // Verify __dir_index dimension exists on root
                let root = vfs.store.read(&vfs.root_id).unwrap();
                assert!(
                    root.dimension("__dir_index").is_some(),
                    "__dir_index not persisted"
                );
            }

            // Phase 2: reopen and verify lookups still work (index loaded from disk)
            {
                let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
                for i in 0..10 {
                    let path = format!("/file{:02}.txt", i);
                    assert!(
                        vfs.lookup(&path).is_ok(),
                        "post-open lookup failed for {}",
                        path
                    );
                }

                // Verify readdir returns all entries
                let entries = vfs.readdir(1).unwrap();
                let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
                for i in 0..10 {
                    assert!(names.contains(&format!("file{:02}.txt", i).as_str()));
                }
            }

            clean(name);
        }

        #[test]
        fn test_vfs_lookup_by_name() {
            let name = "lookup_by_name";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let file_inode = vfs.create(1, "target.txt").unwrap();
            vfs.sync().unwrap();

            let (inode, particle) = vfs.lookup_by_name(1, "target.txt").unwrap();
            assert_eq!(inode, file_inode);
            assert_eq!(particle.name(), Some("target.txt"));

            assert!(vfs.lookup_by_name(1, "missing.txt").is_err());

            clean(name);
        }

        #[test]
        fn test_vfs_rename_cross() {
            let name = "rename_cross";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let src_dir = vfs.mkdir(1, "src").unwrap();
            let dst_dir = vfs.mkdir(1, "dst").unwrap();
            let file_inode = vfs.create(src_dir, "moveme.txt").unwrap();
            vfs.sync().unwrap();

            // Verify file exists in src
            assert!(vfs.lookup_by_name(src_dir, "moveme.txt").is_ok());
            assert!(vfs.lookup_by_name(dst_dir, "moveme.txt").is_err());

            // Move cross-directory with rename
            vfs.rename_cross(src_dir, "moveme.txt", dst_dir, "moved.txt")
                .unwrap();
            vfs.sync().unwrap();

            // Verify moved
            assert!(vfs.lookup_by_name(src_dir, "moveme.txt").is_err());
            let (inode, particle) = vfs.lookup_by_name(dst_dir, "moved.txt").unwrap();
            assert_eq!(inode, file_inode);
            assert_eq!(particle.name(), Some("moved.txt"));

            clean(name);
        }

        #[test]
        fn test_vfs_dir_index_migration() {
            let name = "dir_index_migrate";
            clean(name);

            // Phase 1: create particles directly in a PersistentStore (no VFS = no __dir_index)
            {
                let mut store = PersistentStore::create(&tmp_path(name), 5, "Migrate").unwrap();
                let root_id = ParticleId::from_content(b"__vfs_root__");
                let mut root = Particle::new(root_id.clone());
                root.set_dimension("name", Wavelet::from_string(""));

                for i in 0..5 {
                    let id = ParticleId::from_content(format!("file{}", i).as_bytes());
                    let mut file = Particle::new(id.clone());
                    file.set_dimension("name", Wavelet::from_string(&format!("legacy{}.txt", i)));
                    store.write(file).unwrap();

                    root.add_bond(id, GravityKind::Contains, 1.0);
                }
                store.write(root).unwrap();
                store.sync().unwrap();
            }

            // Phase 2: open with VFS — it should build the index from bonds on first access
            {
                let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
                for i in 0..5 {
                    let path = format!("/legacy{}.txt", i);
                    let (inode, p) = vfs.lookup(&path).unwrap();
                    assert!(inode > 1);
                    assert_eq!(p.name(), Some(format!("legacy{}.txt", i).as_str()));
                }

                // After first access, __dir_index should have been created
                let root = vfs.store.read(&vfs.root_id).unwrap();
                assert!(
                    root.dimension("__dir_index").is_some(),
                    "index not created after migration"
                );
            }

            clean(name);
        }

        #[test]
        fn test_vfs_unlink_deletes_particle() {
            let name = "unlink_delete";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let file_inode = vfs.create(1, "to_delete.txt").unwrap();
            vfs.sync().unwrap();

            // Verify particle exists in store
            let id = vfs.inodes.get_id(file_inode).unwrap().clone();
            assert!(vfs.store.read(&id).is_ok());

            // Unlink should delete the particle
            vfs.unlink(1, "to_delete.txt").unwrap();
            vfs.sync().unwrap();

            // Particle should no longer exist in store
            assert!(vfs.store.read(&id).is_err());
            // And inode should be removed
            assert!(vfs.inodes.get_id(file_inode).is_none());

            clean(name);
        }

        #[test]
        fn test_vfs_rmdir_empty_only() {
            let name = "rmdir_empty";
            clean(name);

            let mut vfs = DefsVfs::open(&tmp_path(name)).unwrap();
            let dir_inode = vfs.mkdir(1, "parent").unwrap();
            vfs.create(dir_inode, "child.txt").unwrap();
            vfs.sync().unwrap();

            // rmdir on non-empty directory should fail
            assert!(vfs.rmdir(1, "parent").is_err());

            // Unlink the child first
            vfs.unlink(dir_inode, "child.txt").unwrap();
            vfs.sync().unwrap();

            // Now rmdir should succeed
            vfs.rmdir(1, "parent").unwrap();
            vfs.sync().unwrap();

            assert!(vfs.lookup("/parent").is_err());

            clean(name);
        }
    }
}

#[cfg(feature = "std")]
pub use std_impl::*;
