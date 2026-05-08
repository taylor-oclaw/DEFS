//! # Storage Backend Trait
//!
//! The interface between DEFS and higher-level systems (VyMatik, AuraOS).
//!
//! VyMatik implements `StorageBackend` for DEFS to use DEFS as its
//! native storage engine. AuraOS implements `Filesystem` for DEFS to
//! be the root filesystem.

use alloc::string::String;
use alloc::vec::Vec;

use crate::particle::{GravityKind, Particle, ParticleId, Wavelet};
use crate::store::{SearchQuery, StoreError};

/// Metrics reported by a storage backend
#[derive(Clone, Debug, Default)]
pub struct BackendMetrics {
    pub backend_name: String,
    pub total_bytes_written: u64,
    pub total_bytes_read: u64,
    pub total_particles_stored: u64,
    pub total_dimensions_stored: u64,
    pub total_gravity_bonds: u64,
    pub avg_write_latency_us: u64,
    pub avg_read_latency_us: u64,
    pub avg_search_latency_us: u64,
    pub dedup_ratio: f32,
    pub compression_ratio: f32,
    pub cache_hit_rate: f32,
    pub disk_usage_bytes: u64,
}

/// Core storage backend interface.
///
/// VyMatik implements this for RocksDB (now) and DEFS (later).
/// DEFS implements this for its own PersistentStore.
pub trait StorageBackend {
    /// Backend identifier
    fn name(&self) -> &str;

    /// Particle CRUD
    fn write(&mut self, particle: &Particle) -> Result<ParticleId, StoreError>;
    fn read(&self, id: &ParticleId) -> Result<Particle, StoreError>;
    fn delete(&mut self, id: &ParticleId) -> Result<(), StoreError>;
    fn exists(&self, id: &ParticleId) -> bool;

    /// Dimension-level access (columnar)
    fn read_dimension(
        &self,
        id: &ParticleId,
        dimension: &str,
    ) -> Result<Option<Wavelet>, StoreError>;

    fn write_dimension(
        &mut self,
        id: &ParticleId,
        dimension: &str,
        wavelet: &Wavelet,
    ) -> Result<(), StoreError>;

    /// Gravity traversal
    fn outgoing_bonds(
        &self,
        id: &ParticleId,
        kind: Option<GravityKind>,
    ) -> Result<Vec<crate::particle::GravityBond>, StoreError>;

    fn incoming_bonds(
        &self,
        id: &ParticleId,
        kind: Option<GravityKind>,
    ) -> Result<Vec<(ParticleId, crate::particle::GravityBond)>, StoreError>;

    /// Search
    fn search(&self, query: &SearchQuery) -> Result<Vec<Particle>, StoreError>;

    /// Find particles semantically similar to a query string
    fn search_semantic(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<(ParticleId, f32)>, StoreError>;

    /// Find particles semantically similar to another particle
    fn search_similar(
        &self,
        id: &ParticleId,
        k: usize,
    ) -> Result<Vec<(ParticleId, f32)>, StoreError>;

    /// Scan all particles
    fn scan(&self) -> Result<Vec<Particle>, StoreError>;

    /// Transactions
    fn begin_transaction(&mut self) -> Result<TransactionHandle, StoreError>;
    fn commit(&mut self, txn: TransactionHandle) -> Result<(), StoreError>;
    fn rollback(&mut self, txn: TransactionHandle) -> Result<(), StoreError>;

    /// Snapshots
    fn snapshot(&mut self, label: &str) -> Result<u64, StoreError>;
    fn restore_snapshot(&mut self, snapshot_id: u64) -> Result<(), StoreError>;

    /// Sync to disk
    fn sync(&mut self) -> Result<(), StoreError>;

    /// Metrics
    fn metrics(&self) -> BackendMetrics;
}

/// Opaque transaction handle
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TransactionHandle(pub u64);

/// Filesystem interface for OS integration (AuraOS)
///
/// This is the POSIX-compatible layer that AuraOS uses.
/// DEFS implements this via its FUSE driver or kernel module.
pub trait Filesystem {
    fn name(&self) -> &str;

    // POSIX operations
    fn open(&mut self, path: &str, flags: u32) -> Result<u64, FsError>;
    fn read(&mut self, fd: u64, buf: &mut [u8], offset: u64) -> Result<usize, FsError>;
    fn write(&mut self, fd: u64, buf: &[u8], offset: u64) -> Result<usize, FsError>;
    fn close(&mut self, fd: u64) -> Result<(), FsError>;
    fn mkdir(&mut self, path: &str, mode: u16) -> Result<(), FsError>;
    fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError>;
    fn stat(&mut self, path: &str) -> Result<FileStat, FsError>;
    fn unlink(&mut self, path: &str) -> Result<(), FsError>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), FsError>;

    // Particle-native operations
    fn read_particle(&self, id: &ParticleId) -> Result<Particle, FsError>;
    fn write_particle(&mut self, particle: &Particle) -> Result<(), FsError>;
    fn find_by_intent(&self, intent: &str) -> Result<Vec<ParticleId>, FsError>;
}

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub inode: u64,
    pub entry_type: u8,
}

#[derive(Clone, Debug)]
pub struct FileStat {
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
}

/// Model-aware storage interface (Patent #2, #3)
///
/// Specialized operations for AI model storage:
/// - Layer-addressable access
/// - Delta storage for fine-tunes
/// - KV cache persistence
pub trait ModelStorage {
    /// Store a base model
    fn store_base_model(
        &mut self,
        model_id: &str,
        layers: &[(String, Vec<u8>)],
    ) -> Result<(), StoreError>;

    /// Store a fine-tuned model (delta from base)
    fn store_finetune(
        &mut self,
        model_id: &str,
        base_model_id: &str,
        changed_layers: &[(String, Vec<u8>)],
    ) -> Result<(), StoreError>;

    /// Load a specific layer
    fn load_layer(&self, model_id: &str, layer_name: &str) -> Result<Vec<u8>, StoreError>;

    /// Store KV cache for a session
    fn store_kv_cache(
        &mut self,
        session_id: &str,
        model_id: &str,
        layer_caches: &[(String, Vec<u8>)],
    ) -> Result<(), StoreError>;

    /// Load KV cache for a session
    fn load_kv_cache(
        &self,
        session_id: &str,
        model_id: &str,
    ) -> Result<Vec<(String, Vec<u8>)>, StoreError>;

    /// Get model metadata
    fn model_info(&self, model_id: &str) -> Result<ModelInfo, StoreError>;
}

#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub model_id: String,
    pub base_model: Option<String>,
    pub total_layers: usize,
    pub total_size_bytes: u64,
    pub delta_size_bytes: u64,
    pub compression_ratio: f32,
}

// ------------------------------------------------------------------
// Async backend (feature-gated)
// ------------------------------------------------------------------

#[cfg(feature = "async-backend")]
pub mod async_impl {
    use alloc::string::String;
    use alloc::vec::Vec;
    use async_trait::async_trait;
    use std::sync::Mutex;

    use crate::backend::{BackendMetrics, StorageBackend, TransactionHandle};
    use crate::particle::{GravityKind, Particle, ParticleId, Wavelet};
    use crate::persist::PersistentStore;
    use crate::store::{SearchQuery, StoreError};

    /// Async mirror of `StorageBackend`.
    ///
    /// All methods are `async` so DEFS can be driven from async runtimes.
    /// The current implementation delegates to the synchronous store inside
    /// a `std::sync::Mutex` — true async I/O is a future optimization.
    #[async_trait]
    pub trait AsyncStorageBackend {
        async fn name(&self) -> String;
        async fn write(&self, particle: &Particle) -> Result<ParticleId, StoreError>;
        async fn read(&self, id: &ParticleId) -> Result<Particle, StoreError>;
        async fn delete(&self, id: &ParticleId) -> Result<(), StoreError>;
        async fn exists(&self, id: &ParticleId) -> bool;
        async fn read_dimension(
            &self,
            id: &ParticleId,
            dimension: &str,
        ) -> Result<Option<Wavelet>, StoreError>;
        async fn write_dimension(
            &self,
            id: &ParticleId,
            dimension: &str,
            wavelet: &Wavelet,
        ) -> Result<(), StoreError>;
        async fn outgoing_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<crate::particle::GravityBond>, StoreError>;
        async fn incoming_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<(ParticleId, crate::particle::GravityBond)>, StoreError>;
        async fn search(&self, query: &SearchQuery) -> Result<Vec<Particle>, StoreError>;
        async fn search_semantic(
            &self,
            query: &str,
            k: usize,
        ) -> Result<Vec<(ParticleId, f32)>, StoreError>;
        async fn search_similar(
            &self,
            id: &ParticleId,
            k: usize,
        ) -> Result<Vec<(ParticleId, f32)>, StoreError>;
        async fn scan(&self) -> Result<Vec<Particle>, StoreError>;
        async fn begin_transaction(&self) -> Result<TransactionHandle, StoreError>;
        async fn commit(&self, txn: TransactionHandle) -> Result<(), StoreError>;
        async fn rollback(&self, txn: TransactionHandle) -> Result<(), StoreError>;
        async fn snapshot(&self, label: &str) -> Result<u64, StoreError>;
        async fn restore_snapshot(&self, snapshot_id: u64) -> Result<(), StoreError>;
        async fn sync(&self) -> Result<(), StoreError>;
        async fn metrics(&self) -> BackendMetrics;
    }

    /// Wrapper around `PersistentStore` that exposes an async interface.
    ///
    /// Uses `std::sync::Mutex` for interior mutability.  All operations
    /// are synchronous under the hood — the async surface is strictly for
    /// runtime compatibility.
    pub struct AsyncPersistentStore {
        inner: Mutex<PersistentStore>,
    }

    impl AsyncPersistentStore {
        pub fn new(store: PersistentStore) -> Self {
            Self {
                inner: Mutex::new(store),
            }
        }

        /// Load all particles from disk into memory.
        pub fn load_all(&self) -> Result<usize, crate::volume::VolumeError> {
            let mut guard = self.inner.lock().unwrap();
            guard.load_all()
        }
    }

    #[async_trait]
    impl AsyncStorageBackend for AsyncPersistentStore {
        async fn name(&self) -> String {
            "defs-persistent".to_string()
        }

        async fn write(&self, particle: &Particle) -> Result<ParticleId, StoreError> {
            let mut guard = self.inner.lock().unwrap();
            let id = particle.id.clone();
            guard.write(particle.clone())?;
            Ok(id)
        }

        async fn read(&self, id: &ParticleId) -> Result<Particle, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.read(id)
        }

        async fn delete(&self, id: &ParticleId) -> Result<(), StoreError> {
            let mut guard = self.inner.lock().unwrap();
            guard
                .delete(id)
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        async fn exists(&self, id: &ParticleId) -> bool {
            let guard = self.inner.lock().unwrap();
            guard.exists(id)
        }

        async fn read_dimension(
            &self,
            id: &ParticleId,
            dimension: &str,
        ) -> Result<Option<Wavelet>, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.read_dimension(id, dimension)
        }

        async fn write_dimension(
            &self,
            id: &ParticleId,
            dimension: &str,
            wavelet: &Wavelet,
        ) -> Result<(), StoreError> {
            let mut guard = self.inner.lock().unwrap();
            guard.write_dimension(id, dimension, wavelet)
        }

        async fn outgoing_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<crate::particle::GravityBond>, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.outgoing_bonds(id, kind)
        }

        async fn incoming_bonds(
            &self,
            id: &ParticleId,
            kind: Option<GravityKind>,
        ) -> Result<Vec<(ParticleId, crate::particle::GravityBond)>, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.incoming_bonds(id, kind)
        }

        async fn search(&self, query: &SearchQuery) -> Result<Vec<Particle>, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.search(query)
        }

        async fn search_semantic(
            &self,
            query: &str,
            k: usize,
        ) -> Result<Vec<(ParticleId, f32)>, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.search_semantic(query, k)
        }

        async fn search_similar(
            &self,
            id: &ParticleId,
            k: usize,
        ) -> Result<Vec<(ParticleId, f32)>, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.search_similar(id, k)
        }

        async fn scan(&self) -> Result<Vec<Particle>, StoreError> {
            let guard = self.inner.lock().unwrap();
            guard.scan()
        }

        async fn begin_transaction(&self) -> Result<TransactionHandle, StoreError> {
            // Stub: no real transaction isolation yet
            Ok(TransactionHandle(1))
        }

        async fn commit(&self, _txn: TransactionHandle) -> Result<(), StoreError> {
            let mut guard = self.inner.lock().unwrap();
            guard
                .sync()
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        async fn rollback(&self, _txn: TransactionHandle) -> Result<(), StoreError> {
            // Stub: would need to track uncommitted changes
            Ok(())
        }

        async fn snapshot(&self, label: &str) -> Result<u64, StoreError> {
            let mut guard = self.inner.lock().unwrap();
            guard
                .snapshot(label)
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        async fn restore_snapshot(&self, snapshot_id: u64) -> Result<(), StoreError> {
            let mut guard = self.inner.lock().unwrap();
            guard
                .restore_snapshot(snapshot_id)
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        async fn sync(&self) -> Result<(), StoreError> {
            let mut guard = self.inner.lock().unwrap();
            guard
                .sync()
                .map_err(|e| StoreError::IoError(format!("{:?}", e)))
        }

        async fn metrics(&self) -> BackendMetrics {
            let guard = self.inner.lock().unwrap();
            guard.metrics()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::particle::{Particle, ParticleId, Wavelet};
        use std::path::PathBuf;

        #[tokio::test]
        async fn test_async_persistent_store_roundtrip() {
            let path = PathBuf::from("/tmp/test_async_persist.defs");
            let _ = std::fs::remove_file(&path);

            // Create, write, sync, and read back via async interface
            {
                let store = PersistentStore::create(&path, 10, "TestAsync").unwrap();
                let async_store = AsyncPersistentStore::new(store);

                let id = ParticleId::from_content(b"async_doc");
                let mut p = Particle::new(id.clone());
                p.set_dimension("name", Wavelet::from_string("async_report.pdf"));
                p.set_dimension("content", Wavelet::from_binary(b"async hello"));
                p.created_at_ns = 67890;

                let written_id = async_store.write(&p).await.unwrap();
                assert_eq!(written_id, id);

                async_store.sync().await.unwrap();

                let read_p = async_store.read(&id).await.unwrap();
                assert_eq!(read_p.name(), Some("async_report.pdf"));
                assert_eq!(
                    read_p.content().unwrap().as_binary(),
                    Some(&b"async hello"[..])
                );
                assert_eq!(read_p.created_at_ns, 67890);

                assert!(async_store.exists(&id).await);

                let metrics = async_store.metrics().await;
                assert_eq!(metrics.backend_name, "defs-persistent");
            }

            // Re-open and load all via async interface
            {
                let store = PersistentStore::open(&path).unwrap();
                let async_store = AsyncPersistentStore::new(store);
                let count = async_store.load_all().unwrap();
                assert_eq!(count, 1);

                let id = ParticleId::from_content(b"async_doc");
                let read_p = async_store.read(&id).await.unwrap();
                assert_eq!(read_p.name(), Some("async_report.pdf"));
            }

            let _ = std::fs::remove_file(&path);
        }
    }
}
