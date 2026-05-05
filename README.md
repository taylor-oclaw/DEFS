# DEFS — Data-Enriched File System v2

> The filesystem that understands your data.

DEFS is an AI-native, content-aware filesystem designed for the age of intelligent agents. Unlike traditional filesystems that treat files as dead blobs of bytes, DEFS stores every file as a **Particle** — a semantically rich, multi-dimensional object with typed properties, relationship graphs, and native AI metadata.

## Core Concepts

```
┌─────────────────────────────────────────────────────────────┐
│  HUMAN VIEW        │  AGENT VIEW        │  DATABASE VIEW    │
│  (POSIX files)     │  (particles)       │  (VyMatik)        │
├─────────────────────────────────────────────────────────────┤
│  report.pdf        │  Particle {        │  Row in table     │
│  /docs/budget.xlsx │    id: blake3(...) │  with columns:    │
│                    │    dimensions: {   │    content,       │
│                    │      "content":    │    name,          │
│                    │        Wavelet,    │    content_type,  │
│                    │      "name":       │    embedding...   │
│                    │        Wavelet,    │                   │
│                    │      "embedding":  │                   │
│                    │        Wavelet     │                   │
│                    │    },              │                   │
│                    │    gravity: [      │  Graph edges:     │
│                    │      Bond→related  │    related_to,    │
│                    │    ]               │    derived_from   │
│                    │  }                 │                   │
└─────────────────────────────────────────────────────────────┘
```

### Particle
The atomic unit of DEFS. Replaces the traditional inode. Every file, directory, symlink, and data fragment is a Particle with:
- **Dimensions** — typed properties (content, name, ACL, embedding, etc.)
- **Gravity bonds** — relationships to other particles (Contains, DependsOn, RelatedTo, etc.)
- **Content hash** — blake3 for integrity and deduplication

### Wavelet
The encoded signal within a Dimension. Self-describing binary format with type tag, payload, and metadata.

### Singularity
A dense cluster of particles where schema emerges. Replaces the traditional directory — not a container, but an emergent grouping based on dimensional correlation.

### Gravity
Defines how particles relate. Typed, weighted bonds enable graph traversal, semantic search, and relationship discovery.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Application / Agent Layer                                   │
│  POSIX │ Agent API │ VyMatik Query                          │
├─────────────────────────────────────────────────────────────┤
│  Prism Layer (Multiple Projections)                          │
│  POSIX Prism │ Agent Prism │ Database Prism                  │
├─────────────────────────────────────────────────────────────┤
│  Particle Store + Gravity Index                              │
├─────────────────────────────────────────────────────────────┤
│  Journal │ CoW │ Deduplication │ Decay │ Prefetch           │
├─────────────────────────────────────────────────────────────┤
│  Block Layer: Allocator │ Compression                        │
├─────────────────────────────────────────────────────────────┤
│  Disk / NVMe / Block Device                                  │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

### Build
```bash
cargo build --release --all --features std
```

### Run tests
```bash
cargo test --all --features std
cargo test --all --features std,async-backend
```

### Create a volume
```bash
./target/release/defs mkfs my-volume.defs --label "My Data"
```

### VFS operations (POSIX-style)
```bash
# List files and directories
./target/release/defs ls my-volume.defs /

# Read file content
./target/release/defs cat my-volume.defs /report.txt

# Create directories
./target/release/defs mkdir my-volume.defs /projects

# Write files
./target/release/defs write my-volume.defs /projects/hello.txt --data "Hello, DEFS!"
./target/release/defs write my-volume.defs /projects/data.bin --input ./data.bin

# Move/rename files
./target/release/defs mv my-volume.defs /projects/hello.txt /projects/greeting.txt

# Remove files and directories
./target/release/defs rm my-volume.defs /projects/greeting.txt
```

### Snapshots
```bash
# Create a snapshot
./target/release/defs snapshot my-volume.defs "before-upgrade"

# List snapshots
./target/release/defs snapshots my-volume.defs

# Restore to a snapshot
./target/release/defs restore my-volume.defs 1
```

### Maintenance
```bash
# Check volume health
./target/release/defs fsck my-volume.defs

# Reclaim leaked blocks
./target/release/defs compact my-volume.defs

# Show volume info and cache stats
./target/release/defs info my-volume.defs
./target/release/defs df my-volume.defs
```

## FUSE Mount (Linux)

```bash
# Build with FUSE support
cargo build -p defs-fuse --features fuse-mount

# Mount as POSIX filesystem
mkdir -p /mnt/defs
./target/release/defs-fuse my-volume.defs /mnt/defs

# Use like a normal filesystem
ls /mnt/defs
cat /mnt/defs/report.txt
cp /mnt/defs/report.txt ~/Desktop/
```

## Project Structure

| Crate | Description |
|---|---|
| `defs-core` | Core filesystem — `no_std` compatible particle store, gravity indexing, dedup, journal |
| `defs-fuse` | FUSE driver for Linux/macOS (optional, requires libfuse3/macFUSE) |
| `defs-cli` | Command-line interface for volume management |

## Features (v1.0)

- ✅ **Content-addressable storage** — blake3 hashing for deduplication
- ✅ **Particle model** — files as semantically rich objects with dimensions
- ✅ **Gravity bonds** — typed, weighted relationships between particles
- ✅ **Columnar dimension access** — read single properties without loading full particles
- ✅ **Graph traversal** — follow gravity bonds with configurable depth
- ✅ **Semantic search** — query by dimension values and content
- ✅ **B+tree directory index** — O(log n) lookup for directory entries
- ✅ **CoW snapshots** — copy-on-write versioning with snapshot/restore
- ✅ **Journal / WAL** — crash-safe write-ahead logging
- ✅ **Block-level deduplication** — content-addressed block sharing with ref counting
- ✅ **Compaction** — reclaim leaked/orphaned blocks
- ✅ **Fsck** — volume integrity checker with repair mode
- ✅ **Block cache** — 64-block FIFO cache with hit/miss metrics
- ✅ **FUSE driver** — POSIX compatibility layer (fsync, chmod, chown)
- ✅ **Async backend** — `AsyncStorageBackend` trait for tokio integration
- ✅ **CLI tool** — mkfs, VFS commands, snapshots, compact, fsck, info
- ✅ **no_std support** — kernel-ready for AuraOS integration
- 🔄 **Intelligence layer** — AI metadata, semantic search, prefetch (planned v1.1)
- 🔄 **Decay policies** — automatic lifecycle management (planned v1.1)

## Comparison

| Feature | ext4 | ZFS | APFS | **DEFS** |
|---|---|---|---|---|
| POSIX files | ✅ | ✅ | ✅ | ✅ |
| Content-addressable | ❌ | ⚠️ dedup | ❌ | ✅ blake3 |
| Semantic metadata | ❌ | ❌ | ❌ | ✅ native |
| Relationship graph | ❌ | ❌ | ❌ | ✅ gravity |
| AI model awareness | ❌ | ❌ | ❌ | ✅ layer-addressable |
| Agent-native API | ❌ | ❌ | ❌ | ✅ |
| Columnar access | ❌ | ❌ | ❌ | ✅ dimension read |
| Persistent KV cache | ❌ | ❌ | ❌ | ✅ |
| no_std / kernel | ❌ | ❌ | ❌ | ✅ |

## Patents

Novel inventions in DEFS are patent-pending through Suvayar LLC. See [PATENTS.md](PATENTS.md).

## License

MIT OR Apache-2.0 (core filesystem)
Commercial license required for AI features (semantic tags, model store, prefetch, decay).
