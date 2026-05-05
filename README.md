<div align="center">

# DEFS

### The Filesystem That Understands Your Data

**v1.0.0** — AI-native, content-aware storage with semantic relationships, block-level deduplication, and POSIX compatibility.

[![Tests](https://img.shields.io/badge/tests-88%2F88%20passing-success)](https://github.com/taylor-oclaw/DEFS/actions)
[![Version](https://img.shields.io/badge/version-1.0.0-blue)](https://github.com/taylor-oclaw/DEFS/releases/tag/v1.0.0)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-orange)](./PATENTS.md)

[🌐 Marketing Site](./website/) · [⚙️ Engine](./engine/) · [📖 Architecture](./engine/ARCHITECTURE.md) · [📝 Changelog](./CHANGELOG.md)

</div>

---

## What is DEFS?

DEFS (Data-Enriched File System) reimagines storage for the age of intelligent agents. Instead of treating files as dead blobs of bytes, DEFS stores every file as a **Particle** — a semantically rich object with:

- **Dimensions** — typed properties (content, name, permissions, embeddings)
- **Gravity Bonds** — relationships between files (Contains, DependsOn, RelatedTo, etc.)
- **Content Hashing** — blake3 for integrity and automatic deduplication
- **CoW Snapshots** — copy-on-write versioning built-in

You see normal files. DEFS sees meaning.

```
HUMAN VIEW          AGENT VIEW
─────────────       ──────────────────────────
report.pdf    →     Particle {
                      id: blake3(...),
                      dimensions: {
                        "content": Wavelet,
                        "name": "report.pdf",
                        "content_type": "application/pdf"
                      },
                      gravity: [
                        Contains → /projects
                      ]
                    }
```

## Performance

| Metric | DEFS | SQLite | Advantage |
|---|---|---|---|
| Write 1K particles | ~221ms | ~671ms | **3× faster** |
| Read 1K particles | ~219ns | ~6.43µs | **29× faster** |

## Quick Start

```bash
# Install the CLI
cargo install --path engine/defs-cli --features std

# Create a volume
defs mkfs my-volume.defs --label "My Data"

# Use it like a filesystem
defs mkdir my-volume.defs /projects
defs write my-volume.defs /projects/hello.txt --data "Hello, DEFS!"
defs cat my-volume.defs /projects/hello.txt
```

Or mount via FUSE:

```bash
cargo build --release -p defs-fuse --features fuse-mount
./target/release/defs-fuse my-volume.defs /mnt/defs
ls /mnt/defs/projects/
```

## Project Structure

```
DEFS/
├── engine/          # Rust workspace — core filesystem, CLI, FUSE driver
│   ├── defs-core/   # Core particle store, VFS, block layer
│   ├── defs-cli/    # Command-line interface
│   ├── defs-fuse/   # FUSE mount driver
│   └── ARCHITECTURE.md
├── website/         # Marketing site (static HTML/CSS/JS)
├── CHANGELOG.md
└── PATENTS.md
```

## Features (v1.0)

- ✅ **Particle-based storage** — semantically rich files with dimensions
- ✅ **Gravity bonds** — typed relationships between particles
- ✅ **Block-level deduplication** — content-addressed sharing with ref counting
- ✅ **CoW snapshots** — snapshot, restore, list
- ✅ **WAL crash recovery** — write-ahead logging
- ✅ **B+tree directory index** — O(log n) lookups
- ✅ **Compaction & fsck** — reclaim leaks, repair volumes
- ✅ **FUSE driver** — full POSIX compatibility
- ✅ **Async backend** — tokio-compatible API
- ✅ **no_std support** — kernel-ready for AuraOS

## Testing

```bash
cd engine
cargo test --all --features std          # 88 tests
cargo test --all --features std,async-backend  # 89 tests
```

## License

- **Core filesystem** (defs-core, defs-cli, defs-fuse): MIT OR Apache-2.0
- **AI features** (semantic tags, model store, prefetch, decay): Commercial license required

Patent-pending through Suvayar LLC. See [PATENTS.md](./PATENTS.md) for the full portfolio.

---

<div align="center">

Built with Rust 🦀

</div>
