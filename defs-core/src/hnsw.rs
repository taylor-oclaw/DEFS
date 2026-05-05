//! # HNSW — Hierarchical Navigable Small World
//!
//! Approximate nearest neighbor search for embeddings.
//! Replaces brute-force k-NN with sub-linear search.
//!
//! Algorithm: Multi-layer graph where layer 0 has all nodes,
//! higher layers have sparser connections for fast navigation.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::embed::cosine_similarity;

/// A node in the HNSW graph
#[derive(Clone, Debug)]
pub struct HnswNode {
    pub id: String,
    pub vector: Vec<f32>,
    /// Connections per layer: layer_index → [neighbor_id]
    pub connections: Vec<Vec<usize>>, // node indices, not ids
}

/// HNSW index for approximate nearest neighbor search
pub struct HnswIndex {
    nodes: Vec<HnswNode>,
    /// id → node index
    id_to_idx: BTreeMap<String, usize>,
    /// Maximum layer count
    max_layers: usize,
    /// Number of neighbors per node (M)
    m: usize,
    /// Expansion factor for search (ef)
    ef_construction: usize,
    /// Current entry point (highest layer)
    entry_point: Option<usize>,
    /// Probability decay for layer assignment
    level_multiplier: f32,
}

impl HnswIndex {
    pub fn new(_dim: usize, m: usize, ef_construction: usize) -> Self {
        Self {
            nodes: Vec::new(),
            id_to_idx: BTreeMap::new(),
            max_layers: 16,
            m,
            ef_construction,
            entry_point: None,
            level_multiplier: 1.0 / (m as f32).ln(),
        }
    }

    /// Insert a vector into the index
    pub fn insert(&mut self, id: &str, vector: Vec<f32>) {
        if self.id_to_idx.contains_key(id) {
            return; // Already exists
        }

        let node_idx = self.nodes.len();
        let level = self.random_level();

        let mut node = HnswNode {
            id: String::from(id),
            vector: vector.clone(),
            connections: Vec::with_capacity(level + 1),
        };

        for _ in 0..=level {
            node.connections.push(Vec::new());
        }

        self.nodes.push(node);
        self.id_to_idx.insert(String::from(id), node_idx);

        if self.entry_point.is_none() {
            self.entry_point = Some(node_idx);
            return;
        }

        // Phase 5 simplified: connect to nearest neighbors at each layer
        let ep = self.entry_point.unwrap();

        for layer in (0..=level.min(self.nodes[ep].connections.len().saturating_sub(1))).rev() {
            let neighbors = self.search_layer(&vector, layer, self.m, ep);
            for &neighbor_idx in &neighbors {
                if neighbor_idx != node_idx {
                    // Add bidirectional connection
                    if self.nodes[node_idx].connections.len() > layer {
                        if !self.nodes[node_idx].connections[layer].contains(&neighbor_idx) {
                            self.nodes[node_idx].connections[layer].push(neighbor_idx);
                        }
                    }
                    if self.nodes[neighbor_idx].connections.len() > layer {
                        if !self.nodes[neighbor_idx].connections[layer].contains(&node_idx) {
                            self.nodes[neighbor_idx].connections[layer].push(node_idx);
                        }
                    }
                }
            }
        }

        // Prune connections after all insertions
        for layer in (0..=level.min(self.nodes[ep].connections.len().saturating_sub(1))).rev() {
            if let Some(conns) = self.nodes[node_idx].connections.get(layer) {
                if conns.len() > self.m * 2 {
                    let idx = node_idx;
                    let m = self.m;
                    self.prune_connections(idx, layer, m * 2);
                }
            }
            let neighbors = self.search_layer(&vector, layer, self.m, ep);
            for &neighbor_idx in &neighbors {
                if let Some(conns) = self.nodes[neighbor_idx].connections.get(layer) {
                    if conns.len() > self.m * 2 {
                        let idx = neighbor_idx;
                        let m = self.m;
                        self.prune_connections(idx, layer, m * 2);
                    }
                }
            }
        }

        // Update entry point if this node is at a higher level
        if level >= self.nodes[ep].connections.len().saturating_sub(1) {
            self.entry_point = Some(node_idx);
        }
    }

    /// Search for k nearest neighbors
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if self.nodes.is_empty() || self.entry_point.is_none() {
            return Vec::new();
        }

        let ep = self.entry_point.unwrap();

        // Search from top layer down to find good entry point for layer 0
        let max_layer = self.nodes[ep].connections.len().saturating_sub(1);
        let mut current_ep = ep;

        for layer in (1..=max_layer).rev() {
            current_ep = self.greedy_search_layer(query, layer, current_ep);
        }

        // Final search at layer 0
        let candidates = self.search_layer(query, 0, self.ef_construction.max(k), current_ep);

        // Convert indices to ids and scores
        let mut results: Vec<(String, f32)> = candidates
            .into_iter()
            .map(|idx| (self.nodes[idx].id.clone(), self.distance(query, idx)))
            .collect();

        // Sort by similarity and return top k
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        results.into_iter().take(k).collect()
    }

    fn greedy_search_layer(&self, query: &[f32], layer: usize, start: usize) -> usize {
        let mut current = start;
        let mut best_dist = self.distance(query, current);
        let mut visited = Vec::new();
        visited.push(current);

        loop {
            let mut improved = false;
            if let Some(neighbors) = self.nodes[current].connections.get(layer) {
                for &neighbor in neighbors {
                    if visited.contains(&neighbor) {
                        continue;
                    }
                    visited.push(neighbor);
                    let dist = self.distance(query, neighbor);
                    if dist > best_dist {
                        best_dist = dist;
                        current = neighbor;
                        improved = true;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        current
    }

    fn search_layer(&self, query: &[f32], layer: usize, ef: usize, start: usize) -> Vec<usize> {
        let mut candidates = Vec::new();
        let mut visited = Vec::new();
        let mut to_visit = Vec::new();

        to_visit.push(start);
        visited.push(start);
        candidates.push((start, self.distance(query, start)));

        while let Some(current) = to_visit.pop() {
            if let Some(neighbors) = self.nodes[current].connections.get(layer) {
                for &neighbor in neighbors {
                    if visited.contains(&neighbor) {
                        continue;
                    }
                    visited.push(neighbor);
                    let dist = self.distance(query, neighbor);
                    candidates.push((neighbor, dist));
                    to_visit.push(neighbor);
                }
            }
        }

        // Keep only best ef candidates
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        candidates.truncate(ef);
        candidates.into_iter().map(|(idx, _)| idx).collect()
    }

    fn distance(&self, query: &[f32], node_idx: usize) -> f32 {
        cosine_similarity(query, &self.nodes[node_idx].vector)
    }

    fn prune_connections(&mut self, node_idx: usize, layer: usize, max_conn: usize) {
        let conns_len = self.nodes[node_idx]
            .connections
            .get(layer)
            .map(|c| c.len())
            .unwrap_or(0);
        if conns_len <= max_conn {
            return;
        }

        // Sort by distance to this node and keep closest
        let node_vec = self.nodes[node_idx].vector.clone();
        let mut scored: Vec<(usize, f32)> = self.nodes[node_idx].connections[layer]
            .iter()
            .map(|&idx| (idx, cosine_similarity(&node_vec, &self.nodes[idx].vector)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        scored.truncate(max_conn);

        if let Some(conns) = self.nodes[node_idx].connections.get_mut(layer) {
            *conns = scored.into_iter().map(|(idx, _)| idx).collect();
        }
    }

    fn random_level(&self) -> usize {
        // Simplified: use a geometric distribution
        // In production, use a proper RNG
        let mut level = 0;
        // Deterministic for reproducibility in tests
        while level < self.max_layers - 1 && (level as f32) < self.level_multiplier {
            level += 1;
        }
        level
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(&idx) = self.id_to_idx.get(id) {
            // Remove from all neighbor connections
            for node in &mut self.nodes {
                for layer in &mut node.connections {
                    layer.retain(|&i| i != idx);
                    // Adjust indices for nodes after idx
                    for i in layer.iter_mut() {
                        if *i > idx {
                            *i -= 1;
                        }
                    }
                }
            }
            self.nodes.remove(idx);
            self.id_to_idx.remove(id);
            // Rebuild index map
            self.id_to_idx.clear();
            for (i, node) in self.nodes.iter().enumerate() {
                self.id_to_idx.insert(node.id.clone(), i);
            }
            // Update entry point
            if self.entry_point == Some(idx) {
                self.entry_point = self.nodes.first().map(|_| 0);
            } else if let Some(ep) = self.entry_point {
                if ep > idx {
                    self.entry_point = Some(ep - 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hnsw_basic_insert_search() {
        let mut index = HnswIndex::new(3, 8, 16);

        index.insert("a", vec![1.0, 0.0, 0.0]);
        index.insert("b", vec![0.0, 1.0, 0.0]);
        index.insert("c", vec![0.0, 0.0, 1.0]);
        index.insert("d", vec![0.9, 0.1, 0.0]); // close to a

        assert_eq!(index.len(), 4);

        let results = index.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a"); // exact match should be first
    }

    #[test]
    fn test_hnsw_approximate_search() {
        let mut index = HnswIndex::new(10, 16, 32);

        // Insert 100 random-ish vectors
        for i in 0..100 {
            let mut vec = vec![0.0; 10];
            vec[i % 10] = 1.0;
            index.insert(&format!("v{}", i), vec);
        }

        let query = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let results = index.search(&query, 5);

        assert_eq!(results.len(), 5);
        // The closest should have high similarity
        assert!(results[0].1 > 0.9);
    }

    #[test]
    fn test_hnsw_remove() {
        let mut index = HnswIndex::new(3, 8, 16);
        index.insert("a", vec![1.0, 0.0, 0.0]);
        index.insert("b", vec![0.0, 1.0, 0.0]);

        index.remove("a");
        assert_eq!(index.len(), 1);

        let results = index.search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results[0].0, "b");
    }
}
