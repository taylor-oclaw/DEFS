#![cfg_attr(feature = "fuse-mount", allow(unused))]

#[cfg(feature = "fuse-mount")]
mod fuse_impl;

#[cfg(feature = "fuse-mount")]
fn main() {
    fuse_impl::main();
}

#[cfg(not(feature = "fuse-mount"))]
fn main() {
    eprintln!("DEFS FUSE driver");
    eprintln!();
    eprintln!("This binary requires the 'fuse-mount' feature which needs:");
    eprintln!("  - Linux: libfuse3-dev (apt install libfuse3-dev)");
    eprintln!("  - macOS: macFUSE (brew install macfuse pkg-config)");
    eprintln!();
    eprintln!("Build with: cargo build -p defs-fuse --features fuse-mount");
    std::process::exit(1);
}

#[cfg(feature = "fuse-mount")]
mod fuse_impl {
    use defs_core::vfs::{DefsVfs, DirEntry, VfsAttr, VfsError};
    use fuser::{
        FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory,
        ReplyEntry, ReplyOpen, ReplyEmpty, ReplyStatfs, ReplyWrite, Request, FUSE_ROOT_ID,
    };
    use libc::{EACCES, EEXIST, EIO, EISDIR, ENOENT, ENOTDIR};
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    const TTL: Duration = Duration::from_secs(1);
    const BLOCK_SIZE: u64 = 512;

    fn vfs_err_to_errno(e: VfsError) -> i32 {
        match e {
            VfsError::NotFound => ENOENT,
            VfsError::AlreadyExists => EEXIST,
            VfsError::NotDirectory => ENOTDIR,
            VfsError::IsDirectory => EISDIR,
            VfsError::InvalidPath => EIO,
            VfsError::Io(_) => EIO,
        }
    }

    fn vfs_attr_to_fuse(attr: VfsAttr) -> FileAttr {
        let now = SystemTime::now();
        let created = UNIX_EPOCH + Duration::from_nanos(attr.created_ns);
        let modified = UNIX_EPOCH + Duration::from_nanos(attr.modified_ns);
        let mode = if attr.is_dir { attr.perm | 0o40000 } else { attr.perm | 0o100000 };
        FileAttr {
            ino: attr.inode,
            size: attr.size,
            blocks: (attr.size + BLOCK_SIZE - 1) / BLOCK_SIZE,
            atime: now,
            mtime: modified,
            ctime: modified,
            crtime: created,
            kind: if attr.is_dir { FileType::Directory } else { FileType::RegularFile },
            perm: mode,
            nlink: 1,
            uid: attr.uid,
            gid: attr.gid,
            rdev: 0,
            flags: 0,
            blksize: BLOCK_SIZE as u32,
        }
    }

    struct DefsFs {
        vfs: DefsVfs,
    }

    impl DefsFs {
        fn new(vfs: DefsVfs) -> Self {
            Self { vfs }
        }

        fn load_from_path(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
            let vfs = DefsVfs::open(path)?;
            Ok(Self::new(vfs))
        }
    }

    impl Filesystem for DefsFs {
        fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
            let name = match name.to_str() {
                Some(n) => n,
                None => {
                    reply.error(EIO);
                    return;
                }
            };

            match self.vfs.lookup_by_name(parent, name) {
                Ok((inode, particle)) => {
                    // Build attr from particle directly since lookup_by_name doesn't return VfsAttr
                    let attr = match self.vfs.getattr(inode) {
                        Ok(a) => vfs_attr_to_fuse(a),
                        Err(e) => {
                            reply.error(vfs_err_to_errno(e));
                            return;
                        }
                    };
                    reply.entry(&TTL, &attr, 0);
                }
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn getattr(&mut self, _req: &Request, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
            match self.vfs.getattr(ino) {
                Ok(attr) => reply.attr(&TTL, &vfs_attr_to_fuse(attr)),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn setattr(
            &mut self,
            _req: &Request,
            ino: u64,
            mode: Option<u32>,
            uid: Option<u32>,
            gid: Option<u32>,
            size: Option<u64>,
            _atime: Option<fuser::TimeOrNow>,
            _mtime: Option<fuser::TimeOrNow>,
            _ctime: Option<SystemTime>,
            _fh: Option<u64>,
            _crtime: Option<SystemTime>,
            _chgtime: Option<SystemTime>,
            _bkuptime: Option<SystemTime>,
            _flags: Option<u32>,
            reply: ReplyAttr,
        ) {
            if let Some(size) = size {
                if let Err(e) = self.vfs.truncate(ino, size) {
                    reply.error(vfs_err_to_errno(e));
                    return;
                }
            }

            let mode = mode.map(|m| (m & 0o7777) as u16);
            let uid = uid;
            let gid = gid;
            if mode.is_some() || uid.is_some() || gid.is_some() {
                if let Err(e) = self.vfs.setattr(ino, mode, uid, gid) {
                    reply.error(vfs_err_to_errno(e));
                    return;
                }
            }

            match self.vfs.getattr(ino) {
                Ok(attr) => reply.attr(&TTL, &vfs_attr_to_fuse(attr)),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn read(
            &mut self,
            _req: &Request,
            _ino: u64,
            fh: u64,
            offset: i64,
            size: u32,
            _flags: i32,
            _lock_owner: Option<u64>,
            reply: ReplyData,
        ) {
            if let Err(e) = self.vfs.seek(fh, offset as u64) {
                reply.error(vfs_err_to_errno(e));
                return;
            }

            let mut buf = vec![0u8; size as usize];
            match self.vfs.read(fh, &mut buf) {
                Ok(n) => reply.data(&buf[..n]),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn write(
            &mut self,
            _req: &Request,
            _ino: u64,
            fh: u64,
            offset: i64,
            data: &[u8],
            _write_flags: u32,
            _flags: i32,
            _lock_owner: Option<u64>,
            reply: ReplyWrite,
        ) {
            if let Err(e) = self.vfs.seek(fh, offset as u64) {
                reply.error(vfs_err_to_errno(e));
                return;
            }

            match self.vfs.write(fh, data) {
                Ok(n) => reply.written(n as u32),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn readdir(
            &mut self,
            _req: &Request,
            ino: u64,
            _fh: u64,
            offset: i64,
            mut reply: ReplyDirectory,
        ) {
            let entries = match self.vfs.readdir(ino) {
                Ok(e) => e,
                Err(e) => {
                    reply.error(vfs_err_to_errno(e));
                    return;
                }
            };

            let mut fuse_entries = vec![
                (ino, FileType::Directory, "."),
                (1u64, FileType::Directory, ".."),
            ];

            for entry in &entries {
                if entry.name == "." || entry.name == ".." {
                    continue;
                }
                let ftype = if entry.is_dir { FileType::Directory } else { FileType::RegularFile };
                fuse_entries.push((entry.inode, ftype, entry.name.as_str()));
            }

            for (i, (ino, kind, name)) in fuse_entries.into_iter().enumerate().skip(offset as usize) {
                let full = reply.add(ino, (i + 1) as i64, kind, name);
                if full {
                    break;
                }
            }
            reply.ok();
        }

        fn mkdir(
            &mut self,
            _req: &Request,
            parent: u64,
            name: &OsStr,
            _mode: u32,
            _umask: u32,
            reply: ReplyEntry,
        ) {
            let name = match name.to_str() {
                Some(n) => n,
                None => {
                    reply.error(EIO);
                    return;
                }
            };

            match self.vfs.mkdir(parent, name) {
                Ok(inode) => {
                    match self.vfs.getattr(inode) {
                        Ok(attr) => reply.entry(&TTL, &vfs_attr_to_fuse(attr), 0),
                        Err(e) => reply.error(vfs_err_to_errno(e)),
                    }
                }
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn mknod(
            &mut self,
            _req: &Request,
            parent: u64,
            name: &OsStr,
            _mode: u32,
            _umask: u32,
            _rdev: u32,
            reply: ReplyEntry,
        ) {
            let name = match name.to_str() {
                Some(n) => n,
                None => {
                    reply.error(EIO);
                    return;
                }
            };

            match self.vfs.create(parent, name) {
                Ok(inode) => {
                    match self.vfs.getattr(inode) {
                        Ok(attr) => reply.entry(&TTL, &vfs_attr_to_fuse(attr), 0),
                        Err(e) => reply.error(vfs_err_to_errno(e)),
                    }
                }
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            let name = match name.to_str() {
                Some(n) => n,
                None => {
                    reply.error(EIO);
                    return;
                }
            };

            match self.vfs.unlink(parent, name) {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            let name = match name.to_str() {
                Some(n) => n,
                None => {
                    reply.error(EIO);
                    return;
                }
            };

            match self.vfs.rmdir(parent, name) {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn rename(
            &mut self,
            _req: &Request,
            parent: u64,
            name: &OsStr,
            newparent: u64,
            newname: &OsStr,
            _flags: u32,
            reply: ReplyEmpty,
        ) {
            let name = match name.to_str() {
                Some(n) => n,
                None => {
                    reply.error(EIO);
                    return;
                }
            };
            let newname = match newname.to_str() {
                Some(n) => n,
                None => {
                    reply.error(EIO);
                    return;
                }
            };

            let result = if parent == newparent {
                self.vfs.rename(parent, name, newname)
            } else {
                self.vfs.rename_cross(parent, name, newparent, newname)
            };

            match result {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
            // Simple read/write detection from flags
            let write = (flags & libc::O_WRONLY) != 0 || (flags & libc::O_RDWR) != 0;
            let read = (flags & libc::O_WRONLY) == 0;

            match self.vfs.open_handle(ino, read, write) {
                Ok(handle) => reply.opened(handle, 0),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn release(
            &mut self,
            _req: &Request,
            _ino: u64,
            fh: u64,
            _flags: i32,
            _lock_owner: Option<u64>,
            _flush: bool,
            reply: ReplyEmpty,
        ) {
            match self.vfs.close_handle(fh) {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(vfs_err_to_errno(e)),
            }
        }

        fn flush(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
            if let Err(e) = self.vfs.sync() {
                reply.error(vfs_err_to_errno(e));
                return;
            }
            reply.ok();
        }

        fn fsync(&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) {
            if let Err(e) = self.vfs.sync() {
                reply.error(vfs_err_to_errno(e));
                return;
            }
            reply.ok();
        }

        fn opendir(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
            // Use inode as directory handle — readdir doesn't need a special handle
            reply.opened(ino, 0);
        }

        fn releasedir(
            &mut self,
            _req: &Request,
            _ino: u64,
            _fh: u64,
            _flags: i32,
            reply: ReplyEmpty,
        ) {
            reply.ok();
        }

        fn statfs(&mut self, _req: &Request, _ino: u64, reply: ReplyStatfs) {
            let info = self.vfs.info();
            reply.statfs(
                info.total_blocks,
                info.free_blocks,
                info.free_blocks,               // available to non-superuser
                0,                               // files total (not tracked)
                0,                               // files free
                info.block_size as u32,
                255,                             // max filename length
                0,                               // fragment size
            );
        }
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().collect();
        if args.len() < 3 {
            eprintln!("Usage: defs-fuse <volume.defs> <mountpoint>");
            std::process::exit(1);
        }

        let volume_path = PathBuf::from(&args[1]);
        let mountpoint = &args[2];

        let fs = match DefsFs::load_from_path(&volume_path) {
            Ok(fs) => fs,
            Err(e) => {
                eprintln!("Failed to load volume: {}", e);
                std::process::exit(1);
            }
        };

        println!("Mounting DEFS volume: {} -> {}", volume_path.display(), mountpoint);

        let options = vec![
            MountOption::RW,
            MountOption::FSName("defs".to_string()),
            MountOption::AutoUnmount,
            MountOption::AllowOther,
        ];

        fuser::mount2(fs, mountpoint, &options).unwrap();
    }
}
