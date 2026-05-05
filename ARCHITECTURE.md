# DEFS Architecture Specification

> Data-Enriched File System — The filesystem that understands your data.

## Design Philosophy

DEFS stores data as **Particles** — intelligent, self-describing units of content — while presenting a standard **POSIX file/folder interface** to users and applications. Every existing tool (git, VS Code, vim, rsync, Docker) works unchanged. Power users unlock AI-native capabilities through the `defs` CLI.

**Dual backronym:**
- **Data-Enriched File System** (standalone)
- **Dimensional Encoding File System** (VyMatik integration)

---

## Core Architecture

```
┌──────────────────────────────────────────────────────┐
│              User / Application Layer                 │
│  (ls, cat, git, VS Code, vim, rsync — unchanged)     │
├──────────────────────────────────────────────────────┤
│                POSIX Compatibility Layer               │
│  FUSE mount (Linux/macOS) or Kernel VFS (AuraOS)      │
│  Maps: paths ↔ particles, dirs ↔ gravity clusters     │
├──────────────────────────────────────────────────────┤
│                   Particle Engine                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────┐   │
│  │ Particle  │  │ Gravity  │  │    Resonance     │   │
│  │ Storage   │  │ Bonds    │  │    Dimensions    │   │
│  └──────────┘  └──────────┘  └──────────────────┘   │
├──────────────────────────────────────────────────────┤
│                Intelligence Layer                      │
│  AI Metadata │ Semantic Search │ Prefetch │ Decay      │
├──────────────────────────────────────────────────────┤
│                  Storage Layer                         │
│  Journal │ B+Tree DirIndex │ Block Cache │ Dedup │ CoW │
│  Allocator │ Compaction │ Fsck │ Async Backend          │
├──────────────────────────────────────────────────────┤
│               Disk / Block Device                      │
└──────────────────────────────────────────────────────┘
```

---

## 1. Particles (Core Storage Unit)

A **Particle** replaces the traditional inode. Every file, directory, symlink, and data fragment is a Particle.

```rust
pub struct Particle {
    /// Unique content-addressable ID (blake3 hash)
    pub id: ParticleId,
    
    /// The actual content bytes
    pub content: ParticleContent,
    
    /// Resonance dimensions (metadata encoding)
    pub resonance: Resonance,
    
    /// Gravity bonds (relationships to other particles)
    pub gravity: Vec<GravityBond>,
    
    /// AI-generated metadata
    pub intelligence: ParticleIntelligence,
    
    /// POSIX compatibility fields
    pub posix: PosixCompat,
    
    /// Version chain
    pub versions: VersionChain,
    
    /// Lifecycle
    pub decay: DecayPolicy,
}
```

### ParticleContent
- **Blob**: Raw bytes (file content)
- **Directory**: Ordered list of child particle references
- **Symlink**: Target path + particle reference
- **Model**: AI model with layer-addressable storage
- **Stream**: Append-only log (for real-time data)

### ParticleId
- Content-addressable: `blake3(content + resonance)`
- Automatic deduplication: identical content = same particle
- Immutable reference: ID never changes for same content

---

## 2. Gravity Bonds (Relationships)

**Gravity** defines how particles relate to each other. Unlike traditional hard links or directory entries, gravity bonds are typed and weighted.

```rust
pub struct GravityBond {
    /// Target particle
    pub target: ParticleId,
    
    /// Bond type
    pub kind: GravityKind,
    
    /// Bond strength (0.0 - 1.0)
    /// Stronger bonds = closer relationship
    pub strength: f32,
    
    /// Optional label
    pub label: Option<String>,
}

pub enum GravityKind {
    /// Hierarchical containment (parent → child, like directories)
    Contains,
    
    /// Code dependency (imports, uses)
    DependsOn,
    
    /// Semantic similarity (AI-detected)
    RelatedTo,
    
    /// Version chain (previous → next)
    VersionOf,
    
    /// Reference (documentation → code, test → implementation)
    References,
    
    /// Composition (module → submodules)
    ComposedOf,
    
    /// Custom user-defined bond
    Custom(String),
}
```

### Path Mapping via Gravity

Traditional file paths map to gravity chains:

```
/projects/HireFlow360/src/main.rs

Equivalent gravity path:
root ──Contains──▶ projects ──Contains──▶ HireFlow360 
  ──Contains──▶ src ──Contains──▶ main.rs

Each segment is a Directory particle with a Contains bond 
to child particles, preserving exact POSIX path semantics.
```

### Beyond Paths

Gravity bonds also express things paths can't:

```
main.rs ──DependsOn──▶ config.rs      (import relationship)
main.rs ──RelatedTo──▶ auth_test.rs   (AI: related logic)
main.rs ──VersionOf──▶ main.rs@v2     (previous version)
README.md ──References──▶ main.rs     (documentation link)
```

---

## 3. Resonance Dimensions (Metadata Encoding)

**Resonance** encodes what a particle IS — its type, format, encoding, and semantic properties. Inspired by VyMatik's 7-dimensional encoding.

```rust
pub struct Resonance {
    /// Content type (rust, markdown, image, model, config, etc.)
    pub content_type: ContentType,
    
    /// Encoding (utf8, binary, compressed, encrypted)
    pub encoding: Encoding,
    
    /// Semantic role (source, test, config, doc, asset, data)
    pub role: SemanticRole,
    
    /// AI-generated tags (["authentication", "middleware", "async"])
    pub tags: Vec<String>,
    
    /// Language (for code: rust/python/ts; for text: en/es/zh)
    pub language: Option<String>,
    
    /// Quality score (0.0 - 1.0, AI-assessed)
    pub quality: f32,
    
    /// Sensitivity level (public, internal, confidential, secret)
    pub sensitivity: Sensitivity,
}
```

---

## 4. FUSE Compatibility Layer (POSIX on Top)

**This is the critical adoption layer.** Users see standard files and folders. DEFS translates every POSIX operation into particle operations.

### POSIX → Particle Translation Table

| POSIX Operation | Particle Operation |
|---|---|
| `open(path)` | Resolve gravity chain → find particle → return content |
| `read(fd, buf)` | Read particle content bytes |
| `write(fd, buf)` | Create new particle version (CoW), update gravity |
| `mkdir(path)` | Create Directory particle + Contains bond to parent |
| `readdir(path)` | Query all Contains bonds from directory particle |
| `stat(path)` | Return particle.posix (size, times, mode) |
| `unlink(path)` | Remove Contains bond (particle persists if other bonds exist) |
| `rename(old, new)` | Move Contains bond from old parent to new parent |
| `chmod/chown` | Update particle.posix fields |
| `symlink` | Create Symlink particle with References bond |
| `link` | Add additional Contains bond (multiple parents) |

### FUSE Mount

```bash
# Mount DEFS volume
defs mount /dev/sda1 /mnt/defs

# Everything works normally
ls /mnt/defs/projects/
git clone repo /mnt/defs/projects/myapp
vim /mnt/defs/projects/myapp/src/main.rs
cargo build --manifest-path /mnt/defs/projects/myapp/Cargo.toml

# Power user commands (particle-aware)
defs find "authentication logic"       # Semantic search across ALL particles
defs related src/main.rs               # Show all gravity bonds
defs explain src/                      # AI summary of directory
defs versions src/main.rs              # Version history with AI changelogs
defs bonds src/main.rs                 # Show dependency graph
defs tag src/main.rs "critical-path"   # Add custom tag
defs decay src/tmp/ --after 30d        # Auto-delete after 30 days
```

### Background Intelligence

When files are written through FUSE, DEFS asynchronously:
1. Computes blake3 hash → dedup check
2. Detects content type + encoding (resonance)
3. Queues AI analysis (via local LLM) → generates tags, role, quality
4. Builds gravity bonds → detects imports, references, similarities
5. Updates embedding index → enables semantic search

This happens in the background. The `write()` call returns immediately. Intelligence enrichment is eventual, not blocking.

---

## 5. Intelligence Layer

### AI Metadata Generation
- Uses local LLM (Ollama/llama.cpp) to analyze each particle
- Generates: summary, tags, semantic role, quality score
- Detects: language, framework, dependencies
- Runs asynchronously after write operations

### Semantic Search (Embeddings)
- Every particle gets an embedding vector (nomic-embed-text or similar)
- Stored in a local vector index
- `defs find "query"` → cosine similarity search across all particles
- Cross-project search: find related code across ALL projects

### Predictive Prefetch
- Learns access patterns (when you open main.rs, you usually open config.rs next)
- Pre-loads predicted particles into cache
- Gravity bonds inform prefetch (strong bonds = likely co-access)

### Decay Policies
- Automatic lifecycle management
- Configurable per particle or per directory
- `hot → warm → cold → archive → delete`
- Based on access frequency, age, and gravity bond strength

---

## 6. Version Management

Every write creates a new particle version (Copy-on-Write):

```
main.rs@v1 ──VersionOf──▶ main.rs@v2 ──VersionOf──▶ main.rs@v3 (current)
```

- `defs versions main.rs` → shows all versions with timestamps
- `defs diff main.rs v1 v3` → diff between versions
- `defs restore main.rs v1` → restore old version
- AI generates changelog between versions automatically
- Old versions are particles too — they participate in search, bonds, etc.

---

## 7. Integration Modes

### AuraOS Kernel (no_std)
```rust
// Direct kernel integration — no FUSE overhead
defs-core = { version = "0.1", features = ["kernel"] }
// Particles are native kernel objects
// Gravity bonds replace the directory tree
// System calls map directly to particle operations
```

### Linux/macOS (FUSE)
```rust
// Userspace filesystem via FUSE
defs-fuse = "0.1"
// Full POSIX compatibility
// Background intelligence via tokio async
// Local LLM integration for AI features
```

### VyMatik Storage Engine
```rust
// DEFS as storage backend for VyMatik
defs-core = { version = "0.1", features = ["vymatik"] }
// Particles map to VyMatik Resonance encoding
// Gravity bonds map to VyMatik Gravity
// "Dimensional Encoding File System" backronym
```

---

## 8. Implementation Status (v1.0)

| Phase | Module | File | Status | Description |
|---|---|---|---|---|
| 1 | Particle | `particle.rs` | ✅ | Core Particle struct, ParticleId, Wavelet dimensions |
| 2 | Gravity | `particle.rs` | ✅ | Gravity bonds, bond types, path resolution |
| 3 | Store | `store.rs` | ✅ | In-memory particle store with search/query |
| 4 | Volume | `volume.rs` | ✅ | On-disk block I/O, bitmap allocator, superblock |
| 5 | Format | `format.rs` | ✅ | Page headers, serialization, checksums |
| 6 | WAL | `wal.rs` | ✅ | Write-ahead log for crash recovery |
| 7 | Persist | `persist.rs` | ✅ | PersistentStore: particle ↔ volume bridge |
| 8 | B+tree Index | `dir_index.rs` | ✅ | Directory index for O(log n) lookups |
| 9 | VFS | `vfs.rs` | ✅ | Virtual filesystem: paths, inodes, file handles |
| 10 | Backend | `backend.rs` | ✅ | `StorageBackend` + `Filesystem` traits, async variant |
| 11 | FUSE | `defs-fuse/` | ✅ | FUSE mount driver with full POSIX ops |
| 12 | Deduplication | `dedup.rs` | ✅ | Content-addressable block dedup with ref counting |
| 13 | Snapshots | `persist.rs` | ✅ | CoW snapshots with dedup table preservation |
| 14 | Compaction | `persist.rs` | ✅ | Reclaim leaked/orphaned blocks |
| 15 | Fsck | `fsck.rs` | ✅ | Volume integrity checker + repair |
| 16 | CLI | `defs-cli/` | ✅ | mkfs, VFS ops, snapshots, compact, fsck, info |
| 17 | Intelligence | — | 🔄 | AI metadata, LLM integration (planned v1.1) |
| 18 | Embeddings | — | 🔄 | Vector index, semantic search (planned v1.1) |
| 19 | Prefetch | — | 🔄 | Access pattern learning (planned v1.1) |
| 20 | Decay | — | 🔄 | Lifecycle policies (planned v1.1) |

---

## 9. Patents (Suvayar LLC)

Novel inventions in DEFS — see [PATENTS.md](PATENTS.md) for the full portfolio of 13 inventions including:
- Particle-based content-addressable filesystem
- Gravity bond relationship system
- Resonance-dimensional metadata encoding
- AI-powered semantic file search
- Predictive prefetch via gravity analysis
- Decay-based lifecycle management
- FUSE translation layer for particle ↔ POSIX mapping

---

## 10. Comparison

| Feature | ext4 | NTFS | ZFS | Btrfs | **DEFS** |
|---|---|---|---|---|---|
| Journaling | ✅ | ✅ | ✅ | ✅ | ✅ |
| Snapshots | ❌ | ❌ | ✅ | ✅ | ✅ (per-particle CoW) |
| Dedup | ❌ | ❌ | ✅ | ❌ | ✅ (content-addressable) |
| Compression | ❌ | ✅ | ✅ | ✅ | ✅ |
| Semantic tags | ❌ | ❌ | ❌ | ❌ | ✅ |
| AI metadata | ❌ | ❌ | ❌ | ❌ | ✅ |
| Relationship tracking | ❌ | ❌ | ❌ | ❌ | ✅ (Gravity) |
| Semantic search | ❌ | ❌ | ❌ | ❌ | ✅ |
| Predictive prefetch | ❌ | ❌ | ❌ | ❌ | ✅ |
| Decay/lifecycle | ❌ | ❌ | ❌ | ❌ | ✅ |
| Model-aware | ❌ | ❌ | ❌ | ❌ | ✅ |
| POSIX compatible | ✅ | ✅ | ✅ | ✅ | ✅ (via FUSE) |
