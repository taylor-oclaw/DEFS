//! # Embedding Index
//!
//! Vector similarity search for semantic queries.
//! Phase 2: Brute-force k-NN. Phase 5: HNSW or similar approximate index.

use alloc::string::String;
use alloc::vec::Vec;

/// Compute cosine similarity between two vectors
/// Returns value in [-1.0, 1.0]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

/// Compute Euclidean distance between two vectors
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::MAX;
    }
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// A single entry in the embedding index
#[derive(Clone, Debug)]
pub struct EmbeddingEntry {
    pub id: String, // Particle ID as hex string
    pub vector: Vec<f32>,
    pub dimension_name: String,
}

/// Embedding index with HNSW approximate search
/// Phase 5: HNSW for sub-linear search
pub struct EmbeddingIndex {
    entries: Vec<EmbeddingEntry>,
    hnsw: crate::hnsw::HnswIndex,
    dim: usize,
    use_hnsw: bool,
}

impl EmbeddingIndex {
    pub fn new(dim: usize) -> Self {
        Self {
            entries: Vec::new(),
            hnsw: crate::hnsw::HnswIndex::new(dim, 16, 32),
            dim,
            use_hnsw: true,
        }
    }

    /// Create with brute-force only (for small datasets)
    pub fn new_bruteforce(dim: usize) -> Self {
        Self {
            entries: Vec::new(),
            hnsw: crate::hnsw::HnswIndex::new(dim, 16, 32),
            dim,
            use_hnsw: false,
        }
    }

    pub fn insert(&mut self, id: &str, vector: Vec<f32>, dimension_name: &str) {
        if vector.len() != self.dim {
            return;
        }
        self.entries.push(EmbeddingEntry {
            id: String::from(id),
            vector: vector.clone(),
            dimension_name: String::from(dimension_name),
        });
        if self.use_hnsw {
            self.hnsw.insert(id, vector);
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.entries.retain(|e| e.id != id);
        if self.use_hnsw {
            self.hnsw.remove(id);
        }
    }

    /// k-NN search using cosine similarity
    pub fn search_cosine(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if query.len() != self.dim {
            return Vec::new();
        }

        if self.use_hnsw && self.hnsw.len() >= 50 {
            // Use HNSW for large datasets
            self.hnsw.search(query, k)
        } else {
            // Brute force for small datasets
            let mut scored: Vec<(String, f32)> = self
                .entries
                .iter()
                .map(|e| (e.id.clone(), cosine_similarity(query, &e.vector)))
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal));
            scored.into_iter().take(k).collect()
        }
    }

    /// k-NN search using Euclidean distance
    pub fn search_euclidean(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if query.len() != self.dim {
            return Vec::new();
        }

        let mut scored: Vec<(String, f32)> = self
            .entries
            .iter()
            .map(|e| (e.id.clone(), euclidean_distance(query, &e.vector)))
            .collect();

        scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal));
        scored.into_iter().take(k).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn hnsw_size(&self) -> usize {
        self.hnsw.len()
    }
}

/// Quantize float32 vector to uint8 (for storage efficiency)
pub fn quantize_u8(vec: &[f32]) -> Vec<u8> {
    vec.iter()
        .map(|&v| ((v + 1.0) * 127.5).clamp(0.0, 255.0) as u8)
        .collect()
}

/// Dequantize uint8 vector back to float32
pub fn dequantize_u8(vec: &[u8]) -> Vec<f32> {
    vec.iter().map(|&v| (v as f32 / 127.5) - 1.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.0001);
    }

    #[test]
    fn test_embedding_index_knn() {
        let mut index = EmbeddingIndex::new(3);
        index.insert("p1", vec![1.0, 0.0, 0.0], "embedding");
        index.insert("p2", vec![0.0, 1.0, 0.0], "embedding");
        index.insert("p3", vec![0.9, 0.1, 0.0], "embedding");

        let results = index.search_cosine(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "p1"); // closest
        assert_eq!(results[1].0, "p3"); // second closest
    }

    #[test]
    fn test_quantize_roundtrip() {
        let original = vec![-1.0, -0.5, 0.0, 0.5, 1.0];
        let quantized = quantize_u8(&original);
        let recovered = dequantize_u8(&quantized);
        for (a, b) in original.iter().zip(recovered.iter()) {
            assert!(
                (a - b).abs() < 0.02,
                "Quantization error too large: {} vs {}",
                a,
                b
            );
        }
    }
}
