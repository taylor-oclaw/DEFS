# Changelog

All notable changes to DEFS (Data-Enriched File System) are documented in this file.

## [1.1.0] — 2026-05-07

### Added — Semantic Search
- **HNSW vector index** — Hierarchical Navigable Small World approximate nearest neighbor search with proper geometric distribution RNG
- **Cosine & Euclidean distance** — Both similarity metrics supported; sorts descending for cosine, ascending for euclidean
- **`ef_search` parameter** — Separate from `ef_construction` for query-time recall tuning
- **HashSet visited tracking** — O(1) lookups instead of O(n) linear scan
- **Serialization** — Compact binary format for index persistence via `to_bytes()` / `from_bytes()`
- **CLI `defs find`** — Semantic search by query string with `--semantic` flag; falls back to dimension-contains
- **CLI `defs similar`** — Find k-most-similar particles to a given path
- **`build_embedding_index()`** — On-demand vector indexing from particle embeddings
- **`search_semantic()`** / **`search_similar()`** — PersistentStore APIs for programmatic access

### Added — Web Dashboard
- **`defs-dashboard`** — New binary crate; zero-dependency HTTP server on configurable port (default 8765)
- **Real-time volume stats** — Particle count, singularities, dimensions histogram, bond kinds, size/usage
- **Particle browser** — Sortable table with ID, name, type, dimensions, bonds, incoming, modified time
- **Interactive gravity graph** — Force-directed Canvas visualization with drag, zoom, hover tooltips; color-coded by type (directory, text, image, file)
- **Search interface** — Contains, Equals, and Related-to search with dimension selector
- **Single-page app** — Embedded HTML/CSS/JS; no external CDN dependencies

### Added — API Improvements
- **`ParticleId::from_hex()`** — Parse 64-char hex strings back to ParticleId
- **`PersistentStore::singularity_count()`** — Expose singularity count for dashboards

### Fixed
- **HNSW `random_level()` bug** — Was deterministic; now uses proper geometric distribution with `rand::thread_rng()`
- **HNSW entry point healing** — After removing the entry point, finds the highest-layer node instead of defaulting to index 0

## [1.0.0] — 2026-05-04

### Added — Core Storage
- **Particle model** — Files stored as semantically rich particles with dimensions, gravity bonds, and content hashes
- **Wavelet dimensions** — Self-describing binary format for typed properties (content, name, ACL, etc.)
- **Gravity bonds** — Typed, weighted relationships between particles (Contains, DependsOn, RelatedTo, VersionOf, References, ComposedOf)
- **PersistentStore** — Bridges in-memory ParticleStore with on-disk Volume via WAL and particle index
- **Write-ahead log (WAL)** — Crash-safe journaling with recovery replay
- **B+tree directory index** — `__dir_index` binary dimension for O(log n) directory lookups; auto-migrates from bond scans

### Added — Block Layer
- **Volume manager** — Block I/O, superblock, bitmap allocator
- **Block cache** — 64-block FIFO cache with hit/miss metrics exposed in `VolumeInfo`
- **Block-level deduplication** — Content-addressed block sharing via blake3 hashing with reference counting
- **Multi-page dimensions** — Chained blocks for dimensions >4KB, backward compatible with single-page layout
- **Page headers with CRC32 checksums** — Integrity verification on every block read

### Added — Snapshots & Compaction
- **CoW snapshots** — Freeze particle index copy; snapshot, restore, and list operations
- **Compaction** — Rewrite all indexed particles to reclaim leaked/orphaned blocks
- **Fsck** — Volume integrity checker with repair mode:
  - Superblock validity
  - Page header & checksum integrity
  - Orphaned block detection
  - Dangling bond detection and repair
  - Dedup table consistency
  - Snapshot table integrity
  - Particle index consistency
  - Orphaned particle detection

### Added — VFS & POSIX
- **DefsVfs** — Virtual filesystem with inode table, lazy loading, and path resolution
- **POSIX operations** — open, read, write, close, mkdir, readdir, stat, unlink, rmdir, rename, truncate, setattr (chmod/chown)
- **Lazy loading** — Particles loaded on-demand via `ensure_particle_loaded()`
- **File handle offsets** — Proper offset advancement on read/write

### Added — FUSE Driver
- **FUSE mount** — Full POSIX compatibility via `defs-fuse` binary
- **FUSE operations** — lookup, getattr, setattr, read, write, readdir, mkdir, mknod, unlink, rmdir, rename, open, release, flush, fsync, opendir, releasedir, statfs
- **fsync/flush** — Data durability through `vfs.sync()`
- **chmod/chown support** — Permission and owner attributes stored as wavelet dimensions

### Added — Async Backend
- **AsyncStorageBackend trait** — Feature-gated `async-backend` with full CRUD, dimensions, gravity, search, sync, and metrics
- **AsyncPersistentStore** — Wraps sync store in `Mutex` with async-compatible API

### Added — CLI
- `defs mkfs` — Create a new DEFS volume
- `defs ls` — List directory contents
- `defs cat` — Read file contents
- `defs mkdir` — Create a directory
- `defs write` — Write data to a file (from string or file input)
- `defs rm` — Remove a file or directory
- `defs mv` — Move/rename a file or directory
- `defs snapshot` — Create a snapshot
- `defs restore` — Restore to a snapshot
- `defs snapshots` — List all snapshots
- `defs compact` — Compact volume to reclaim leaked blocks
- `defs fsck` — Check volume integrity (with `--repair`)
- `defs info` — Show volume information and cache stats
- `defs df` — Show disk usage
- `defs particle add/get/list` — Low-level particle operations
- `defs search` — Search particles by dimension content
- `defs bonds` — Show gravity bonds for a particle
- `defs enrich` — Enrich particles with AI-generated metadata
- `defs sync` — Sync volume to disk

### Added — Testing
- 88 tests covering particle roundtrip, large dimensions, delete/reclaim, snapshot create/restore, compaction, VFS operations, fsck, dedup consistency, stress tests (10K particles, WAL recovery, on-demand loading, large particles)
- Stress tests run by default (previously `#[ignored]`)
- `tempfile` crate for robust test cleanup

### Fixed
- Bitmap accounting bug — `alloc_block` now uses `bitmap.alloc_one()` so `free_count` is accurate
- File handle offset bug — `read`/`write` now advance `fh.offset` after I/O
- Snapshot restore bug — `load_all()` now skips blocks not in `particle_index`
- VFS unlink particle leak — `unlink` now properly deletes particles from store
- VFS rmdir safety — `rmdir` now checks directory emptiness before deletion
- Compaction + dedup interaction — `dedup_table` cleared and rebuilt during compaction
- Snapshot + dedup preservation — Dedup table copied alongside particle index in snapshots
- Fsck orphaned_particles — Field now properly computed from directory containment graph

### Performance
- DEFS write 1k particles: ~221ms (3× faster than SQLite ~671ms)
- DEFS read 1k particles: ~219ns (29× faster than SQLite ~6.43µs)

---

## [0.1.0] — 2026-05-03

- Initial project structure and architecture specification
