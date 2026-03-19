# DEFS — Data-Enriched File System

> The filesystem that understands your data.

DEFS is an AI-native, content-aware filesystem designed for the age of intelligent agents. Unlike traditional filesystems that treat files as dead blobs of bytes, DEFS enriches every file with semantic metadata, content hashing, relationship tracking, and lifecycle management.

## Why DEFS?

| Feature | ext4 | NTFS | ZFS | **DEFS** |
|---------|------|------|-----|----------|
| Journaling | ✅ | ✅ | ✅ | ✅ |
| Extent-based | ✅ | ❌ | ✅ | ✅ |
| Snapshots | ❌ | ❌ | ✅ | ✅ (CoW) |
| Dedup | ❌ | ❌ | ✅ | ✅ (content-addressable) |
| Semantic tags | ❌ | ❌ | ❌ | ✅ |
| AI model awareness | ❌ | ❌ | ❌ | ✅ |
| Predictive prefetch | ❌ | ❌ | ❌ | ✅ |
| Decay policies | ❌ | ❌ | ❌ | ✅ |
| Layer-addressable models | ❌ | ❌ | ❌ | ✅ |

## Integration

```rust
// Kernel mode (AuraOS)
defs-core = { version = "0.1", features = ["kernel"] }

// FUSE driver (Linux/macOS)
defs-fuse = "0.1"

// VyMatik storage engine
defs-core = { version = "0.1", features = ["vymatik"] }
```

## Architecture

- **defs-core** — `no_std` compatible core (works in kernels and userspace)
- **defs-fuse** — FUSE driver for Linux/macOS/Windows
- **defs-tools** — `mkfs.defs`, `fsck.defs`, `defs-dump` (coming soon)

## How Operating Systems Use DEFS

DEFS is designed as a **portable library**, not tied to any OS:

```
AuraOS kernel → imports defs-core with feature = "kernel"
Linux/macOS   → runs defs-fuse (FUSE userspace driver)
Agent OS      → imports defs-core with feature = "kernel"
VyMatik       → imports defs-core with feature = "vymatik"
```

## Patents

Novel inventions in DEFS are patent-pending through Suvayar LLC. See [PATENTS.md](PATENTS.md).

## License

MIT OR Apache-2.0 (core filesystem)
Commercial license required for AI features (semantic tags, model store, prefetch, decay).
