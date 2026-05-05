//! # Stress Tests
//!
//! Production-readiness torture tests for DEFS.

#[cfg(feature = "std")]
mod std_impl {
    #![allow(unused_imports)]

    use std::time::Instant;

    use crate::particle::{Particle, ParticleId, Wavelet};
    use crate::persist::PersistentStore;

    /// Write N particles, sync, reopen, and verify random reads
    #[test]
    fn stress_write_10k_particles() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("stress.defs");

        let mut store = PersistentStore::create(&path, 500, "Stress").unwrap();
        let count = 10_000;

        let start = Instant::now();
        for i in 0..count {
            let id = ParticleId::from_content(format!("doc{}", i).as_bytes());
            let mut p = Particle::new(id);
            p.set_dimension("name", Wavelet::from_string(&format!("file_{}.txt", i)));
            p.set_dimension("content", Wavelet::from_binary(&vec![(i % 256) as u8; 256]));
            store.write(p).unwrap();
        }
        let write_time = start.elapsed();

        let start = Instant::now();
        store.sync().unwrap();
        let sync_time = start.elapsed();

        println!(
            "Wrote {} particles: {:?} (write), {:?} (sync)",
            count, write_time, sync_time
        );

        // Reopen and read back
        let mut store = PersistentStore::open(&path).unwrap();
        let start = Instant::now();
        let loaded = store.load_all().unwrap();
        let load_time = start.elapsed();
        assert_eq!(loaded, count);
        println!("Loaded {} particles in {:?}", loaded, load_time);

        // Random reads
        let start = Instant::now();
        for i in (0..count).step_by(100) {
            let id = ParticleId::from_content(format!("doc{}", i).as_bytes());
            let p = store.read(&id).unwrap();
            assert_eq!(p.name(), Some(&format!("file_{}.txt", i)[..]));
        }
        let read_time = start.elapsed();
        println!("Random reads: {:?}", read_time);
    }

    /// Test particle index with on-demand loading
    #[test]
    fn stress_index_on_demand_load() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("stress.defs");

        let mut store = PersistentStore::create(&path, 100, "Stress").unwrap();
        let count = 5_000;

        for i in 0..count {
            let id = ParticleId::from_content(format!("doc{}", i).as_bytes());
            let mut p = Particle::new(id);
            p.set_dimension("name", Wavelet::from_string(&format!("file_{}.txt", i)));
            store.write(p).unwrap();
        }
        store.sync().unwrap();

        // Reopen WITHOUT load_all
        let mut store = PersistentStore::open(&path).unwrap();
        assert_eq!(store.particle_count(), 0); // Nothing loaded yet

        // Load specific particles on demand
        let start = Instant::now();
        for i in (0..count).step_by(50) {
            let id = ParticleId::from_content(format!("doc{}", i).as_bytes());
            let p = store.load_particle(&id).unwrap();
            assert_eq!(p.name(), Some(&format!("file_{}.txt", i)[..]));
        }
        let load_time = start.elapsed();
        println!(
            "On-demand loaded {}/{} particles in {:?}",
            count / 50,
            count,
            load_time
        );
    }

    /// Test WAL recovery after simulated crash (no checkpoint)
    #[test]
    fn stress_wal_recovery() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("stress.defs");

        {
            let mut store = PersistentStore::create(&path, 50, "Stress").unwrap();
            for i in 0..1000 {
                let id = ParticleId::from_content(format!("doc{}", i).as_bytes());
                let mut p = Particle::new(id);
                p.set_dimension("name", Wavelet::from_string(&format!("file_{}.txt", i)));
                store.write(p).unwrap();
            }
            // Write WAL entries but DO NOT checkpoint
            // In real crash, WAL would have entries, main data might be partial
            store.sync().unwrap();
        }

        // Reopen — should recover via WAL replay
        let mut store = PersistentStore::open(&path).unwrap();
        let loaded = store.load_all().unwrap();
        assert!(
            loaded >= 1000,
            "Expected >= 1000 particles after WAL recovery, got {}",
            loaded
        );
        println!("WAL recovery: {} particles restored", loaded);
    }

    /// Large particle content (128KB each) to test multi-block chained dimension handling
    #[test]
    fn stress_large_particles() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("stress.defs");

        let mut store = PersistentStore::create(&path, 500, "Stress").unwrap();
        let count = 100;
        let content_size = 128 * 1024; // 128KB, larger than a single page

        let start = Instant::now();
        for i in 0..count {
            let id = ParticleId::from_content(format!("big{}", i).as_bytes());
            let mut p = Particle::new(id);
            p.set_dimension("name", Wavelet::from_string(&format!("big_{}.bin", i)));
            p.set_dimension(
                "content",
                Wavelet::from_binary(&vec![i as u8; content_size]),
            );
            store.write(p).unwrap();
        }
        store.sync().unwrap();
        println!(
            "Wrote {} x {}KB particles in {:?}",
            count,
            content_size / 1024,
            start.elapsed()
        );

        let mut store = PersistentStore::open(&path).unwrap();
        store.load_all().unwrap();

        let start = Instant::now();
        for i in 0..count {
            let id = ParticleId::from_content(format!("big{}", i).as_bytes());
            let p = store.read(&id).unwrap();
            let content = p.content().unwrap().as_binary().unwrap();
            assert_eq!(content.len(), content_size);
            assert_eq!(content[0], i as u8);
        }
        println!(
            "Read {} x {}KB particles in {:?}",
            count,
            content_size / 1024,
            start.elapsed()
        );
    }
}
