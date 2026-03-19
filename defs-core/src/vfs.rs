use alloc::string::String;
use alloc::vec::Vec;
use alloc::string::String;

pub type InodeNum = u64;

pub enum FsError {
    NotFound,
    PermissionDenied,
    DiskFull,
    Exists,
    NotDirectory,
    IsDirectory,
    IoError,
    Corrupted,
}

pub struct FileHandle {
    pub inode: InodeNum,
    pub offset: u64,
    pub readable: bool,
    pub writable: bool,
}

pub struct DefsVfs {
    pub mounted: bool,
    pub read_only: bool,
    pub next_fd: u32,
    pub open_files: Vec<(u32, FileHandle)>,
}

impl DefsVfs {
    pub fn new() -> Self {
        Self {
            mounted: false,
            read_only: false,
            next_fd: 3,
            open_files: Vec::new(),
        }
    }

    pub fn mount(&mut self) {
        self.mounted = true;
    }

    pub fn unmount(&mut self) {
        self.mounted = false;
        self.open_files.clear();
    }

    pub fn open(&mut self, inode: InodeNum, read: bool, write: bool) -> Result<u32, FsError> {
        if !self.mounted {
            return Err(FsError::IoError);
        }
        let fd = self.next_fd;
        self.next_fd += 1;
        self.open_files.push((fd, FileHandle { inode, offset: 0, readable: read, writable: write }));
        Ok(fd)
    }

    pub fn close(&mut self, fd: u32) -> Result<(), FsError> {
        let idx = self.open_files.iter().position(|&(f, _)| f == fd).ok_or(FsError::NotFound)?;
        self.open_files.remove(idx);
        Ok(())
    }

    pub fn seek(&mut self, fd: u32, offset: u64) -> Result<(), FsError> {
        let handle = self.open_files.iter_mut().find(|&&mut (f, _)| f == fd).ok_or(FsError::NotFound)?;
        handle.1.offset = offset;
        Ok(())
    }

    pub fn open_count(&self) -> usize {
        self.open_files.len()
    }
}
