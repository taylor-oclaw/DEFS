use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use defs_core::compress::auto_compress;
use defs_core::embed::EmbeddingIndex;
use defs_core::hnsw::HnswIndex;
use defs_core::particle::{GravityKind, Particle, ParticleId, Wavelet};
use defs_core::persist::PersistentStore;
use defs_core::store::{ParticleStore, SearchQuery};
use defs_core::text::TextIndex;
use rusqlite::Connection;

fn bench_write_particles(c: &mut Criterion) {
    let mut group = c.benchmark_group("particle_write");
    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::new("count", size), size, |b, &size| {
            b.iter(|| {
                let mut store = ParticleStore::new();
                for i in 0..size {
                    let id = ParticleId::from_content(format!("particle_{}", i).as_bytes());
                    let mut p = Particle::new(id);
                    p.set_dimension("name", Wavelet::from_string(&format!("file_{}.txt", i)));
                    p.set_dimension("content", Wavelet::from_binary(b"hello world"));
                    store.write(p).unwrap();
                }
                black_box(store);
            });
        });
    }
    group.finish();
}

fn bench_read_particles(c: &mut Criterion) {
    let mut store = ParticleStore::new();
    let mut ids = Vec::new();
    for i in 0..1000 {
        let id = ParticleId::from_content(format!("particle_{}", i).as_bytes());
        let mut p = Particle::new(id.clone());
        p.set_dimension("name", Wavelet::from_string(&format!("file_{}.txt", i)));
        p.set_dimension("content", Wavelet::from_binary(b"hello world"));
        store.write(p).unwrap();
        ids.push(id);
    }

    c.bench_function("read_random_particle", |b| {
        b.iter(|| {
            let idx = black_box(42);
            let id = &ids[idx % ids.len()];
            black_box(store.read(id).unwrap());
        });
    });
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("search");
    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("dimension_equals", size),
            size,
            |b, &size| {
                let mut store = ParticleStore::new();
                for i in 0..size {
                    let id = ParticleId::from_content(format!("doc_{}", i).as_bytes());
                    let mut p = Particle::new(id);
                    p.set_dimension(
                        "type",
                        Wavelet::from_string(if i % 2 == 0 { "pdf" } else { "docx" }),
                    );
                    store.write(p).unwrap();
                }
                b.iter(|| {
                    let results = store
                        .search(&SearchQuery::DimensionEquals {
                            name: "type".into(),
                            value: Wavelet::from_string("pdf"),
                        })
                        .unwrap();
                    black_box(results);
                });
            },
        );
    }
    group.finish();
}

fn bench_graph_traversal(c: &mut Criterion) {
    let mut store = ParticleStore::new();
    let root_id = ParticleId::from_content(b"root");
    let mut root = Particle::new(root_id);

    for i in 0..100 {
        let child_id = ParticleId::from_content(format!("child_{}", i).as_bytes());
        store.write(Particle::new(child_id.clone())).unwrap();
        root.add_bond(child_id, GravityKind::Contains, 1.0);
    }
    store.write(root).unwrap();

    c.bench_function("graph_traverse_depth_2", |b| {
        b.iter(|| {
            let results = store
                .search(&SearchQuery::RelatedTo {
                    id: root_id,
                    kind: Some(GravityKind::Contains),
                    max_depth: 1,
                })
                .unwrap();
            black_box(results);
        });
    });
}

fn bench_embedding_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("embedding_search");

    for size in [100, 1000, 10000].iter() {
        // Brute force
        group.bench_with_input(BenchmarkId::new("bruteforce", size), size, |b, &size| {
            let mut index = EmbeddingIndex::new_bruteforce(128);
            for i in 0..size {
                let vec: Vec<f32> = (0..128).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
                index.insert(&format!("v{}", i), vec, "embedding");
            }
            let query: Vec<f32> = (0..128).map(|j| (j % 100) as f32 / 100.0).collect();
            b.iter(|| {
                black_box(index.search_cosine(&query, 10));
            });
        });

        // HNSW
        group.bench_with_input(BenchmarkId::new("hnsw", size), size, |b, &size| {
            let mut index = EmbeddingIndex::new(128);
            for i in 0..size {
                let vec: Vec<f32> = (0..128).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
                index.insert(&format!("v{}", i), vec, "embedding");
            }
            let query: Vec<f32> = (0..128).map(|j| (j % 100) as f32 / 100.0).collect();
            b.iter(|| {
                black_box(index.search_cosine(&query, 10));
            });
        });
    }
    group.finish();
}

fn bench_hnsw_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_scaling");
    for size in [100, 500, 1000, 5000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut index = HnswIndex::new(128, 16, 32);
            for i in 0..size {
                let vec: Vec<f32> = (0..128).map(|j| ((i + j) % 100) as f32 / 100.0).collect();
                index.insert(&format!("v{}", i), vec);
            }
            let query: Vec<f32> = (0..128).map(|j| (j % 100) as f32 / 100.0).collect();
            b.iter(|| {
                black_box(index.search(&query, 10));
            });
        });
    }
    group.finish();
}

fn bench_text_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_search");
    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("inverted_index", size),
            size,
            |b, &size| {
                let mut index = TextIndex::new();
                for i in 0..size {
                    let text = format!("document number {} about finance and technology", i);
                    index.index(&format!("p{}", i), "content", &text);
                }
                b.iter(|| {
                    black_box(index.search("finance technology"));
                });
            },
        );
    }
    group.finish();
}

fn bench_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression");

    // Repetitive data (RLE)
    let repetitive = vec![0xAAu8; 10000];
    group.bench_function("rle_10k", |b| {
        b.iter(|| {
            let (compressed, stats) = auto_compress(&repetitive);
            black_box((compressed, stats));
        });
    });

    // Sequential data (Delta)
    let sequential: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
    group.bench_function("delta_10k", |b| {
        b.iter(|| {
            let (compressed, stats) = auto_compress(&sequential);
            black_box((compressed, stats));
        });
    });

    // Text data (Dict)
    let text = b"hello world ".repeat(1000);
    group.bench_function("dict_12k", |b| {
        b.iter(|| {
            let (compressed, stats) = auto_compress(&text);
            black_box((compressed, stats));
        });
    });

    group.finish();
}

fn bench_dedup(c: &mut Criterion) {
    use defs_core::dedup::DedupEngine;

    let mut group = c.benchmark_group("dedup");

    group.bench_function("dedup_1000_unique", |b| {
        b.iter(|| {
            let mut engine = DedupEngine::new();
            for i in 0..1000 {
                let data = format!("unique_block_{}", i);
                engine.store_or_dedup(data.as_bytes(), i as u64);
            }
            black_box(engine);
        });
    });

    group.bench_function("dedup_1000_identical", |b| {
        let data = vec![0u8; 4096];
        b.iter(|| {
            let mut engine = DedupEngine::new();
            for i in 0..1000 {
                engine.store_or_dedup(&data, i as u64);
            }
            black_box(engine);
        });
    });

    group.finish();
}

fn bench_defs_vs_sqlite(c: &mut Criterion) {
    let mut group = c.benchmark_group("defs_vs_sqlite");

    // DEFS persistent write
    group.bench_function("defs_write_1k", |b| {
        b.iter(|| {
            let path = std::path::PathBuf::from("/tmp/bench_defs.defs");
            let _ = std::fs::remove_file(&path);
            let mut store = PersistentStore::create(&path, 100, "Bench").unwrap();
            for i in 0..1000 {
                let id = ParticleId::from_content(format!("doc{}", i).as_bytes());
                let mut p = Particle::new(id);
                p.set_dimension("name", Wavelet::from_string(&format!("file_{}.txt", i)));
                p.set_dimension("content", Wavelet::from_binary(b"hello world"));
                store.write(p).unwrap();
            }
            store.sync().unwrap();
            let _ = std::fs::remove_file(&path);
        });
    });

    // SQLite write
    group.bench_function("sqlite_write_1k", |b| {
        b.iter(|| {
            let path = "/tmp/bench_sqlite.db";
            let _ = std::fs::remove_file(path);
            let conn = Connection::open(path).unwrap();
            conn.execute(
                "CREATE TABLE particles (id BLOB PRIMARY KEY, name TEXT, content BLOB)",
                [],
            )
            .unwrap();
            for i in 0..1000 {
                let id = blake3::hash(format!("doc{}", i).as_bytes())
                    .as_bytes()
                    .to_vec();
                let name = format!("file_{}.txt", i);
                conn.execute(
                    "INSERT INTO particles (id, name, content) VALUES (?1, ?2, ?3)",
                    [
                        &id as &dyn rusqlite::ToSql,
                        &name as &dyn rusqlite::ToSql,
                        b"hello world" as &dyn rusqlite::ToSql,
                    ],
                )
                .unwrap();
            }
            let _ = std::fs::remove_file(path);
        });
    });

    // DEFS read
    group.bench_function("defs_read_1k", |b| {
        let path = std::path::PathBuf::from("/tmp/bench_defs_read.defs");
        let _ = std::fs::remove_file(&path);
        {
            let mut store = PersistentStore::create(&path, 100, "Bench").unwrap();
            for i in 0..1000 {
                let id = ParticleId::from_content(format!("doc{}", i).as_bytes());
                let mut p = Particle::new(id);
                p.set_dimension("name", Wavelet::from_string(&format!("file_{}.txt", i)));
                p.set_dimension("content", Wavelet::from_binary(b"hello world"));
                store.write(p).unwrap();
            }
            store.sync().unwrap();
        }
        let mut store = PersistentStore::open(&path).unwrap();
        store.load_all().unwrap();

        b.iter(|| {
            let idx = black_box(42);
            let id = ParticleId::from_content(format!("doc{}", idx).as_bytes());
            black_box(store.read(&id).unwrap());
        });
        let _ = std::fs::remove_file(&path);
    });

    // SQLite read
    group.bench_function("sqlite_read_1k", |b| {
        let path = "/tmp/bench_sqlite_read.db";
        let _ = std::fs::remove_file(path);
        {
            let conn = Connection::open(path).unwrap();
            conn.execute(
                "CREATE TABLE particles (id BLOB PRIMARY KEY, name TEXT, content BLOB)",
                [],
            )
            .unwrap();
            for i in 0..1000 {
                let id = blake3::hash(format!("doc{}", i).as_bytes())
                    .as_bytes()
                    .to_vec();
                let name = format!("file_{}.txt", i);
                conn.execute(
                    "INSERT INTO particles (id, name, content) VALUES (?1, ?2, ?3)",
                    [
                        &id as &dyn rusqlite::ToSql,
                        &name as &dyn rusqlite::ToSql,
                        b"hello world" as &dyn rusqlite::ToSql,
                    ],
                )
                .unwrap();
            }
        }
        let conn = Connection::open(path).unwrap();

        b.iter(|| {
            let idx = black_box(42);
            let id = blake3::hash(format!("doc{}", idx).as_bytes())
                .as_bytes()
                .to_vec();
            let mut stmt = conn
                .prepare("SELECT name, content FROM particles WHERE id = ?1")
                .unwrap();
            let _ = stmt
                .query_row([&id], |row| {
                    let _name: String = row.get(0)?;
                    let _content: Vec<u8> = row.get(1)?;
                    Ok(())
                })
                .unwrap();
        });
        let _ = std::fs::remove_file(path);
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_write_particles,
    bench_read_particles,
    bench_search,
    bench_graph_traversal,
    bench_embedding_search,
    bench_hnsw_scaling,
    bench_text_search,
    bench_compression,
    bench_dedup,
    bench_defs_vs_sqlite
);
criterion_main!(benches);
