use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use defs_core::fsck::FsckEngine;
use defs_core::intelligence::IntelligenceEngine;
use defs_core::particle::{GravityKind, Particle, ParticleId, Resonance, SemanticRole, Wavelet};
use defs_core::persist::PersistentStore;
use defs_core::store::SearchQuery;
use defs_core::vfs::DefsVfs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "defs")]
#[command(about = "DEFS — Data-Enriched File System CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new DEFS volume
    Mkfs {
        /// Path to the volume file
        path: PathBuf,
        /// Volume size in MB
        #[arg(short, long, default_value = "100")]
        size: u64,
        /// Volume label
        #[arg(short, long)]
        label: Option<String>,
    },
    /// Show volume information
    Info {
        /// Path to the volume file
        path: PathBuf,
    },
    /// Show disk usage (like `df`)
    Df {
        /// Path to the volume file
        path: PathBuf,
    },
    /// Particle operations
    #[command(subcommand)]
    Particle(ParticleCommands),
    /// Search particles
    Search {
        /// Path to the volume file
        path: PathBuf,
        /// Search query (dimension contains)
        query: String,
        /// Dimension name to search
        #[arg(short, long, default_value = "name")]
        dimension: String,
    },
    /// Enrich all particles with AI metadata
    Enrich {
        /// Path to the volume file
        path: PathBuf,
    },
    /// Show gravity bonds for a particle
    Bonds {
        /// Path to the volume file
        path: PathBuf,
        /// Particle ID (hex)
        id: String,
    },
    /// Sync volume to disk
    Sync {
        /// Path to the volume file
        path: PathBuf,
    },
    /// Check volume integrity
    Fsck {
        /// Path to the volume file
        path: PathBuf,
        /// Attempt repairs
        #[arg(short, long)]
        repair: bool,
    },
    /// List directory contents
    Ls {
        /// Path to the volume file
        path: PathBuf,
        /// Directory path inside the volume
        #[arg(default_value = "/")]
        dir: String,
    },
    /// Read file contents
    Cat {
        /// Path to the volume file
        path: PathBuf,
        /// File path inside the volume
        file: String,
    },
    /// Create a directory
    Mkdir {
        /// Path to the volume file
        path: PathBuf,
        /// Directory path to create
        dir: String,
    },
    /// Write data to a file
    Write {
        /// Path to the volume file
        path: PathBuf,
        /// File path inside the volume
        file: String,
        /// Data to write (raw string)
        #[arg(short, long)]
        data: Option<String>,
        /// Read data from a file
        #[arg(short, long)]
        input: Option<PathBuf>,
    },
    /// Remove a file or directory
    Rm {
        /// Path to the volume file
        path: PathBuf,
        /// Target path to remove
        target: String,
    },
    /// Move/rename a file or directory
    Mv {
        /// Path to the volume file
        path: PathBuf,
        /// Source path
        from: String,
        /// Destination path
        to: String,
    },
    /// Create a snapshot of the current volume state
    Snapshot {
        /// Path to the volume file
        path: PathBuf,
        /// Snapshot label
        #[arg(default_value = "manual")]
        label: String,
    },
    /// Restore volume to a previous snapshot
    Restore {
        /// Path to the volume file
        path: PathBuf,
        /// Snapshot ID
        id: u64,
    },
    /// List all snapshots
    Snapshots {
        /// Path to the volume file
        path: PathBuf,
    },
    /// Compact volume — reclaim leaked blocks and defragment
    Compact {
        /// Path to the volume file
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum ParticleCommands {
    /// Add a particle to the volume
    Add {
        /// Path to the volume file
        path: PathBuf,
        /// Particle name
        #[arg(short, long)]
        name: String,
        /// Content file (optional)
        #[arg(short, long)]
        content: Option<PathBuf>,
        /// Content type
        #[arg(short, long, default_value = "application/octet-stream")]
        content_type: String,
        /// Tags (comma-separated)
        #[arg(short, long)]
        tags: Option<String>,
        /// Semantic role
        #[arg(short, long)]
        role: Option<String>,
        /// Link to another particle (gravity bond)
        #[arg(short, long)]
        link: Option<String>,
        /// Bond kind for link
        #[arg(long, default_value = "related-to")]
        link_kind: String,
        /// Auto-enrich with AI metadata
        #[arg(long)]
        enrich: bool,
    },
    /// Get a particle by ID
    Get {
        /// Path to the volume file
        path: PathBuf,
        /// Particle ID (hex)
        id: String,
    },
    /// List all particles
    List {
        /// Path to the volume file
        path: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Mkfs { path, size, label } => cmd_mkfs(path, size, label),
        Commands::Info { path } => cmd_info(path),
        Commands::Df { path } => cmd_df(path),
        Commands::Particle(cmd) => match cmd {
            ParticleCommands::Add {
                path,
                name,
                content,
                content_type,
                tags,
                role,
                link,
                link_kind,
                enrich,
            } => cmd_particle_add(
                path,
                name,
                content,
                content_type,
                tags,
                role,
                link,
                link_kind,
                enrich,
            ),
            ParticleCommands::Get { path, id } => cmd_particle_get(path, id),
            ParticleCommands::List { path } => cmd_particle_list(path),
        },
        Commands::Search {
            path,
            query,
            dimension,
        } => cmd_search(path, query, dimension),
        Commands::Enrich { path } => cmd_enrich(path),
        Commands::Bonds { path, id } => cmd_bonds(path, id),
        Commands::Sync { path } => cmd_sync(path),
        Commands::Fsck { path, repair } => cmd_fsck(path, repair),
        Commands::Ls { path, dir } => cmd_ls(path, dir),
        Commands::Cat { path, file } => cmd_cat(path, file),
        Commands::Mkdir { path, dir } => cmd_mkdir(path, dir),
        Commands::Write {
            path,
            file,
            data,
            input,
        } => cmd_write(path, file, data, input),
        Commands::Rm { path, target } => cmd_rm(path, target),
        Commands::Mv { path, from, to } => cmd_mv(path, from, to),
        Commands::Snapshot { path, label } => cmd_snapshot(path, label),
        Commands::Restore { path, id } => cmd_restore(path, id),
        Commands::Snapshots { path } => cmd_snapshots(path),
        Commands::Compact { path } => cmd_compact(path),
    }
}

fn open_store(path: &PathBuf) -> Result<PersistentStore> {
    if !path.exists() {
        anyhow::bail!(
            "Volume not found: {}. Run `defs mkfs` first.",
            path.display()
        );
    }
    let mut store = PersistentStore::open(path)
        .with_context(|| format!("Failed to open volume: {}", path.display()))?;
    let loaded = store
        .load_all()
        .with_context(|| "Failed to load particles from volume")?;
    if loaded > 0 {
        println!("(Loaded {} particles from disk)", loaded);
    }
    Ok(store)
}

fn cmd_mkfs(path: PathBuf, size: u64, label: Option<String>) -> Result<()> {
    if path.exists() {
        anyhow::bail!("Volume already exists: {}", path.display());
    }

    let label_str = label.as_deref().unwrap_or("DEFS Volume");
    let store = PersistentStore::create(&path, size, label_str)
        .with_context(|| "Failed to create volume")?;

    let info = store.info();
    println!("Created DEFS volume: {}", path.display());
    println!("  Label: {}", info.label);
    println!(
        "  Size: {} MB ({} blocks × {} bytes)",
        size, info.total_blocks, info.block_size
    );
    println!("  Encoding version: {}", info.encoding_version);
    Ok(())
}

fn cmd_info(path: PathBuf) -> Result<()> {
    let store = open_store(&path)?;
    let info = store.info();

    println!("Volume: {}", path.display());
    println!("  Label: {}", info.label);
    println!(
        "  Blocks: {} total, {} free ({}% used)",
        info.total_blocks, info.free_blocks, info.used_percent
    );
    println!("  Block size: {} bytes", info.block_size);
    println!("  Encoding version: {}", info.encoding_version);
    println!("  Particles in memory: {}", store.particle_count());
    let total_cache = info.cache_hits + info.cache_misses;
    if total_cache > 0 {
        println!(
            "  Block cache: {} hits, {} misses ({:.1}% hit rate)",
            info.cache_hits,
            info.cache_misses,
            (info.cache_hits as f64 / total_cache as f64) * 100.0
        );
    }
    Ok(())
}

fn cmd_df(path: PathBuf) -> Result<()> {
    let store = open_store(&path)?;
    let info = store.info();
    let used = info.total_blocks - info.free_blocks;
    let used_bytes = used * info.block_size as u64;
    let total_bytes = info.total_blocks * info.block_size as u64;
    let free_bytes = info.free_blocks * info.block_size as u64;

    println!(
        "Filesystem     {}-blocks      Used Available Use% Mounted on",
        info.block_size
    );
    println!(
        "{:14} {:10} {:10} {:10} {:3}% {}",
        "defs",
        total_bytes / 1024,
        used_bytes / 1024,
        free_bytes / 1024,
        info.used_percent,
        path.display()
    );
    Ok(())
}

fn cmd_particle_add(
    path: PathBuf,
    name: String,
    content: Option<PathBuf>,
    content_type: String,
    tags: Option<String>,
    role: Option<String>,
    link: Option<String>,
    link_kind: String,
    enrich: bool,
) -> Result<()> {
    let mut store = open_store(&path)?;

    let id_seed = match &content {
        Some(p) => {
            std::fs::read(p).with_context(|| format!("Failed to read content: {}", p.display()))?
        }
        None => name.as_bytes().to_vec(),
    };
    let id = ParticleId::from_content(&id_seed);

    let mut particle = Particle::new(id);
    particle.set_dimension("name", Wavelet::from_string(&name));
    particle.set_dimension("content_type", Wavelet::from_string(&content_type));

    if let Some(content_path) = content {
        let data = std::fs::read(&content_path)
            .with_context(|| format!("Failed to read: {}", content_path.display()))?;
        particle.set_dimension("content", Wavelet::from_binary(&data));
        println!("Added {} bytes from {}", data.len(), content_path.display());
    }

    let mut resonance = Resonance::new(&content_type);
    if let Some(r) = role {
        resonance.role = match r.as_str() {
            "source" => SemanticRole::Source,
            "test" => SemanticRole::Test,
            "config" => SemanticRole::Config,
            "documentation" => SemanticRole::Documentation,
            "asset" => SemanticRole::Asset,
            "data" => SemanticRole::Data,
            "model" => SemanticRole::Model,
            "cache" => SemanticRole::Cache,
            _ => SemanticRole::Unknown,
        };
    }
    resonance.apply_to(&mut particle);

    if let Some(tag_str) = tags {
        let tag_list: Vec<String> = tag_str.split(',').map(|s| s.trim().to_string()).collect();
        particle.set_dimension("tags", Wavelet::from_string(&tag_list.join(", ")));
    }

    if let Some(target_hex) = link {
        let target_bytes = hex::decode(&target_hex).with_context(|| "Invalid particle ID hex")?;
        if target_bytes.len() != 32 {
            anyhow::bail!("Particle ID must be 32 bytes (64 hex chars)");
        }
        let mut target_id = [0u8; 32];
        target_id.copy_from_slice(&target_bytes);

        let kind = match link_kind.as_str() {
            "contains" => GravityKind::Contains,
            "depends-on" => GravityKind::DependsOn,
            "related-to" => GravityKind::RelatedTo,
            "version-of" => GravityKind::VersionOf,
            "references" => GravityKind::References,
            "composed-of" => GravityKind::ComposedOf,
            "derived-from" => GravityKind::DerivedFrom,
            _ => GravityKind::RelatedTo,
        };

        particle.add_bond(ParticleId(target_id), kind, 1.0);
    }

    if enrich {
        let engine = IntelligenceEngine::new();
        engine.enrich(&mut particle);
        println!(
            "  AI-enriched: {} auto-tags, quality={:.2}",
            particle
                .dimension("_auto_tags")
                .and_then(|w| w.as_str())
                .map(|s| s.split(',').count())
                .unwrap_or(0),
            particle
                .dimension("_quality")
                .and_then(|w| w.as_int64())
                .unwrap_or(0) as f64
                / 100.0
        );
    }

    store
        .write(particle)
        .with_context(|| "Failed to write particle")?;
    store.sync().with_context(|| "Failed to sync volume")?;

    println!("Created particle {}", id.to_hex());
    Ok(())
}

fn cmd_enrich(path: PathBuf) -> Result<()> {
    let mut store = open_store(&path)?;
    let engine = IntelligenceEngine::new();

    let ids: Vec<ParticleId> = store.all_particles().iter().map(|p| p.id.clone()).collect();
    let mut enriched = 0;

    for id in ids {
        if let Ok(mut particle) = store.read(&id) {
            engine.enrich(&mut particle);
            store.write(particle)?;
            enriched += 1;
        }
    }

    store.sync()?;
    println!("Enriched {} particles with AI metadata", enriched);
    Ok(())
}

fn cmd_particle_get(path: PathBuf, id_hex: String) -> Result<()> {
    let store = open_store(&path)?;
    let id_bytes = hex::decode(&id_hex).with_context(|| "Invalid particle ID hex")?;
    if id_bytes.len() != 32 {
        anyhow::bail!("Particle ID must be 32 bytes (64 hex chars)");
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&id_bytes);

    let particle = store
        .read(&ParticleId(id))
        .with_context(|| "Particle not found")?;

    println!("Particle {}", particle.id.to_hex());
    println!("  Dimensions:");
    for (name, wavelet) in &particle.dimensions {
        let preview = match wavelet.tag {
            defs_core::particle::TypeTag::String => wavelet
                .as_str()
                .map(|s| {
                    if s.len() > 60 {
                        format!("\"{}...\"", &s[..60])
                    } else {
                        format!("\"{}\"", s)
                    }
                })
                .unwrap_or_else(|| "(invalid utf8)".to_string()),
            defs_core::particle::TypeTag::Binary => {
                format!("<binary, {} bytes>", wavelet.payload.len())
            }
            _ => format!("{:?}", wavelet.tag),
        };
        println!("    {}: {}", name, preview);
    }

    if !particle.gravity.is_empty() {
        println!("  Gravity bonds:");
        for bond in &particle.gravity {
            println!(
                "    → {} ({:?}, strength: {:.2})",
                bond.target.to_hex(),
                bond.kind,
                bond.strength
            );
        }
    }

    Ok(())
}

fn cmd_particle_list(path: PathBuf) -> Result<()> {
    let store = open_store(&path)?;

    println!("Particles in {}:", path.display());
    println!("{:<66} {:<20} {:<15}", "ID", "Name", "Type");
    println!("{}", "-".repeat(105));

    let results = store.all_particles();

    for particle in results {
        let name = particle.name().unwrap_or("(unnamed)");
        let ctype = particle
            .dimension("content_type")
            .and_then(|w| w.as_str())
            .unwrap_or("unknown");
        println!(
            "{} {:<20} {:<15}",
            particle.id.to_hex(),
            if name.len() > 20 { &name[..17] } else { name },
            if ctype.len() > 15 {
                &ctype[..12]
            } else {
                ctype
            }
        );
    }

    println!("\nTotal: {} particles", store.particle_count());
    Ok(())
}

fn cmd_search(path: PathBuf, query: String, dimension: String) -> Result<()> {
    let store = open_store(&path)?;

    let results = store.search(&SearchQuery::DimensionContains {
        name: dimension,
        substring: query,
    })?;

    println!("Found {} particle(s):", results.len());
    for particle in results {
        let name = particle.name().unwrap_or("(unnamed)");
        println!("  {} — {}", particle.id.to_hex(), name);
    }

    Ok(())
}

fn cmd_bonds(path: PathBuf, id_hex: String) -> Result<()> {
    let store = open_store(&path)?;
    let id_bytes = hex::decode(&id_hex).with_context(|| "Invalid particle ID hex")?;
    if id_bytes.len() != 32 {
        anyhow::bail!("Particle ID must be 32 bytes (64 hex chars)");
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&id_bytes);
    let pid = ParticleId(id);

    println!("Gravity bonds for {}", id_hex);

    let outgoing = store.outgoing_bonds(&pid, None)?;
    let has_outgoing = !outgoing.is_empty();
    if has_outgoing {
        println!("\n  Outgoing:");
        for bond in &outgoing {
            println!(
                "    → {} ({:?}, strength: {:.2})",
                bond.target.to_hex(),
                bond.kind,
                bond.strength
            );
        }
    }

    let incoming = store.incoming_bonds(&pid, None)?;
    let has_incoming = !incoming.is_empty();
    if has_incoming {
        println!("\n  Incoming:");
        for (src, bond) in &incoming {
            println!(
                "    ← {} ({:?}, strength: {:.2})",
                src.to_hex(),
                bond.kind,
                bond.strength
            );
        }
    }

    if !has_outgoing && !has_incoming {
        println!("  (no bonds)");
    }

    Ok(())
}

fn cmd_sync(path: PathBuf) -> Result<()> {
    let mut store = open_store(&path)?;
    store.sync().with_context(|| "Failed to sync volume")?;
    println!("Volume synced: {}", path.display());
    Ok(())
}

fn cmd_fsck(path: PathBuf, repair: bool) -> Result<()> {
    let engine = FsckEngine::new(repair);
    let report = engine
        .check(&path)
        .with_context(|| format!("Failed to check volume: {}", path.display()))?;

    println!("Volume: {}", report.volume_path);
    println!("  Total blocks: {}", report.total_blocks);
    println!("  Scanned blocks: {}", report.scanned_blocks);
    println!("  Valid pages: {}", report.valid_pages);
    println!("  Corrupted pages: {}", report.corrupted_pages);
    println!("  Orphaned particles: {}", report.orphaned_particles);
    println!("  Dangling bonds: {}", report.dangling_bonds);
    println!("  Bitmap errors: {}", report.bitmap_errors);
    println!("  Orphaned blocks: {}", report.orphaned_blocks);
    println!("  Dedup errors: {}", report.dedup_errors);
    println!("  Snapshot errors: {}", report.snapshot_errors);
    println!("  Index errors: {}", report.index_errors);
    println!("  Repaired: {}", report.repaired);

    if !report.errors.is_empty() {
        println!("\n  Errors:");
        for error in &report.errors {
            println!("    - {}", error);
        }
    }

    if report.is_clean() {
        println!("\n  ✓ Volume is clean");
        Ok(())
    } else {
        println!("\n  ✗ Volume has errors");
        std::process::exit(1);
    }
}

fn open_vfs(path: &PathBuf) -> Result<DefsVfs> {
    if !path.exists() {
        anyhow::bail!(
            "Volume not found: {}. Run `defs mkfs` first.",
            path.display()
        );
    }
    DefsVfs::open(path).with_context(|| format!("Failed to open volume: {}", path.display()))
}

fn cmd_ls(path: PathBuf, dir: String) -> Result<()> {
    let mut vfs = open_vfs(&path)?;
    let (inode, _) = vfs
        .lookup(&dir)
        .with_context(|| format!("Directory not found: {}", dir))?;
    let entries = vfs
        .readdir(inode)
        .with_context(|| format!("Failed to read directory: {}", dir))?;

    for entry in entries {
        let marker = if entry.is_dir { "d" } else { "-" };
        println!("{} {}", marker, entry.name);
    }
    Ok(())
}

fn cmd_cat(path: PathBuf, file: String) -> Result<()> {
    let mut vfs = open_vfs(&path)?;
    let (inode, _) = vfs
        .lookup(&file)
        .with_context(|| format!("File not found: {}", file))?;

    let handle = vfs
        .open_handle(inode, true, false)
        .with_context(|| "Failed to open file")?;

    let mut buf = vec![0u8; 4096];
    loop {
        match vfs.read(handle, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                use std::io::Write;
                std::io::stdout().write_all(&buf[..n])?;
            }
            Err(e) => anyhow::bail!("Read error: {:?}", e),
        }
    }

    vfs.close_handle(handle)?;
    Ok(())
}

fn split_path(path: &str) -> (&str, &str) {
    let path = path.trim_end_matches('/');
    match path.rsplit_once('/') {
        Some(("", name)) => ("/", name),
        Some((parent, name)) => (parent, name),
        None => ("/", path),
    }
}

fn cmd_mkdir(path: PathBuf, dir: String) -> Result<()> {
    let mut vfs = open_vfs(&path)?;

    let (parent_path, name) = split_path(&dir);
    if name.is_empty() {
        anyhow::bail!("Directory already exists: {}", dir);
    }

    let (parent_inode, _) = vfs
        .lookup(parent_path)
        .with_context(|| format!("Parent directory not found: {}", parent_path))?;

    vfs.mkdir(parent_inode, name)
        .with_context(|| format!("Failed to create directory: {}", dir))?;
    vfs.sync()?;
    println!("Created directory: {}", dir);
    Ok(())
}

fn cmd_write(
    path: PathBuf,
    file: String,
    data: Option<String>,
    input: Option<PathBuf>,
) -> Result<()> {
    let mut vfs = open_vfs(&path)?;

    let content = if let Some(input_path) = input {
        std::fs::read(&input_path)
            .with_context(|| format!("Failed to read input file: {}", input_path.display()))?
    } else if let Some(data_str) = data {
        data_str.into_bytes()
    } else {
        // Read from stdin
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)?;
        buf
    };

    let (parent_path, name) = split_path(&file);

    let file_inode = if let Ok((ino, _)) = vfs.lookup(&file) {
        // File exists — overwrite
        ino
    } else {
        // Need to create the file
        let (parent_inode, _) = vfs
            .lookup(parent_path)
            .with_context(|| format!("Parent directory not found: {}", parent_path))?;
        vfs.create(parent_inode, name)?
    };

    let handle = vfs.open_handle(file_inode, false, true)?;
    vfs.truncate(file_inode, 0)?;
    vfs.seek(handle, 0)?;

    let mut offset = 0usize;
    while offset < content.len() {
        let chunk = &content[offset..(offset + 4096).min(content.len())];
        let written = vfs.write(handle, chunk)?;
        offset += written;
    }

    vfs.close_handle(handle)?;
    vfs.sync()?;
    println!("Wrote {} bytes to {}", content.len(), file);
    Ok(())
}

fn cmd_rm(path: PathBuf, target: String) -> Result<()> {
    let mut vfs = open_vfs(&path)?;

    let (parent_path, name) = split_path(&target);
    if name.is_empty() {
        anyhow::bail!("Cannot remove root directory");
    }

    let (parent_inode, _) = vfs
        .lookup(parent_path)
        .with_context(|| format!("Parent directory not found: {}", parent_path))?;

    // Check if target is a directory
    let (_, target_particle) = vfs.lookup(&target)?;
    let is_dir = !target_particle
        .bonds_by_kind(GravityKind::Contains)
        .is_empty();

    if is_dir {
        vfs.rmdir(parent_inode, name)?;
    } else {
        vfs.unlink(parent_inode, name)?;
    }
    vfs.sync()?;
    println!("Removed: {}", target);
    Ok(())
}

fn cmd_mv(path: PathBuf, from: String, to: String) -> Result<()> {
    let mut vfs = open_vfs(&path)?;

    let (from_parent_path, from_name) = split_path(&from);
    if from_name.is_empty() {
        anyhow::bail!("Cannot move root directory");
    }

    let (to_parent_path, to_name) = split_path(&to);
    if to_name.is_empty() {
        anyhow::bail!("Destination already exists: {}", to);
    }

    let (from_parent_inode, _) = vfs
        .lookup(from_parent_path)
        .with_context(|| format!("Source parent not found: {}", from_parent_path))?;
    let (to_parent_inode, _) = vfs
        .lookup(to_parent_path)
        .with_context(|| format!("Destination parent not found: {}", to_parent_path))?;

    if from_parent_inode == to_parent_inode {
        vfs.rename(from_parent_inode, from_name, to_name)?;
    } else {
        vfs.rename_cross(from_parent_inode, from_name, to_parent_inode, to_name)?;
    }

    vfs.sync()?;
    println!("Moved {} → {}", from, to);
    Ok(())
}

fn cmd_snapshot(path: PathBuf, label: String) -> Result<()> {
    let mut store = open_store(&path)?;
    let id = store
        .snapshot(&label)
        .with_context(|| "Failed to create snapshot")?;
    println!("Created snapshot {}: '{}'", id, label);
    Ok(())
}

fn cmd_restore(path: PathBuf, id: u64) -> Result<()> {
    let mut store = open_store(&path)?;
    store
        .restore_snapshot(id)
        .with_context(|| format!("Failed to restore snapshot {}", id))?;
    println!("Restored snapshot {}", id);
    Ok(())
}

fn cmd_snapshots(path: PathBuf) -> Result<()> {
    let mut store = open_store(&path)?;
    let snaps = store
        .list_snapshots()
        .with_context(|| "Failed to list snapshots")?;

    if snaps.is_empty() {
        println!("No snapshots");
        return Ok(());
    }

    println!("Snapshots:");
    for snap in snaps {
        let dt = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(snap.created_at_ns);
        println!("  {} — '{}' at {:?}", snap.id, snap.label, dt);
    }
    Ok(())
}

fn cmd_compact(path: PathBuf) -> Result<()> {
    let mut store = open_store(&path)?;
    let before = store.info();
    let used_before = before.total_blocks - before.free_blocks;

    let (count, reclaimed) = store.compact().with_context(|| "Compaction failed")?;

    let after = store.info();
    let used_after = after.total_blocks - after.free_blocks;

    println!("Compacted {} particles", count);
    println!(
        "  Reclaimed {} blocks ({:.2} MB)",
        reclaimed,
        (reclaimed * after.block_size as u64) as f64 / (1024.0 * 1024.0)
    );
    println!("  Before: {} blocks used", used_before);
    println!("  After:  {} blocks used", used_after);
    Ok(())
}
