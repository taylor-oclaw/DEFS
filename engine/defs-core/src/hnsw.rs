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

use hashbrown::HashSet;

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
    /// Expansion factor for construction (ef)
    ef_construction: usize,
    /// Expansion factor for search (ef)
    ef_search: usize,
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
            ef_search: ef_construction,
            entry_point: None,
            level_multiplier: 1.0 / (m as f32).ln(),
        }
    }

    /// Set the search-time expansion factor
    pub fn set_ef_search(&mut self, ef_search: usize) {
        self.ef_search = ef_search;
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

    /// Search for k nearest neighbors using cosine similarity
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
        let candidates = self.search_layer(query, 0, self.ef_search.max(k), current_ep);

        // Convert indices to ids and scores
        let mut results: Vec<(String, f32)> = candidates
            .into_iter()
            .map(|idx| (self.nodes[idx].id.clone(), self.distance(query, idx)))
            .collect();

        // Sort by similarity and return top k
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        results.into_iter().take(k).collect()
    }

    /// Search for k nearest neighbors using euclidean distance
    pub fn search_euclidean(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        if self.nodes.is_empty() || self.entry_point.is_none() {
            return Vec::new();
        }

        let ep = self.entry_point.unwrap();

        // Search from top layer down to find good entry point for layer 0
        let max_layer = self.nodes[ep].connections.len().saturating_sub(1);
        let mut current_ep = ep;

        for layer in (1..=max_layer).rev() {
            current_ep = self.greedy_search_layer_euclidean(query, layer, current_ep);
        }

        // Final search at layer 0
        let candidates = self.search_layer_euclidean(query, 0, self.ef_search.max(k), current_ep);

        // Convert indices to ids and scores
        let mut results: Vec<(String, f32)> = candidates
            .into_iter()
            .map(|idx| (self.nodes[idx].id.clone(), self.euclidean_distance(query, idx)))
            .collect();

        // Sort by distance (ascending) and return top k
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        results.into_iter().take(k).collect()
    }

    fn greedy_search_layer(&self, query: &[f32], layer: usize, start: usize) -> usize {
        let mut current = start;
        let mut best_dist = self.distance(query, current);
        let mut visited = HashSet::new();
        visited.insert(current);

        loop {
            let mut improved = false;
            if let Some(neighbors) = self.nodes[current].connections.get(layer) {
                for &neighbor in neighbors {
                    if visited.contains(&neighbor) {
                        continue;
                    }
                    visited.insert(neighbor);
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

    fn greedy_search_layer_euclidean(&self, query: &[f32], layer: usize, start: usize) -> usize {
        let mut current = start;
        let mut best_dist = self.euclidean_distance(query, current);
        let mut visited = HashSet::new();
        visited.insert(current);

        loop {
            let mut improved = false;
            if let Some(neighbors) = self.nodes[current].connections.get(layer) {
                for &neighbor in neighbors {
                    if visited.contains(&neighbor) {
                        continue;
                    }
                    visited.insert(neighbor);
                    let dist = self.euclidean_distance(query, neighbor);
                    if dist < best_dist {
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
        let mut visited = HashSet::new();
        let mut to_visit = Vec::new();

        to_visit.push(start);
        visited.insert(start);
        candidates.push((start, self.distance(query, start)));

        while let Some(current) = to_visit.pop() {
            if let Some(neighbors) = self.nodes[current].connections.get(layer) {
                for &neighbor in neighbors {
                    if visited.contains(&neighbor) {
                        continue;
                    }
                    visited.insert(neighbor);
                    let dist = self.distance(query, neighbor);
                    candidates.push((neighbor, dist));
                    to_visit.push(neighbor);
                }
            }
        }

        // Keep only best ef candidates (descending for cosine)
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        candidates.truncate(ef);
        candidates.into_iter().map(|(idx, _)| idx).collect()
    }

    fn search_layer_euclidean(&self, query: &[f32], layer: usize, ef: usize, start: usize) -> Vec<usize> {
        let mut candidates = Vec::new();
        let mut visited = HashSet::new();
        let mut to_visit = Vec::new();

        to_visit.push(start);
        visited.insert(start);
        candidates.push((start, self.euclidean_distance(query, start)));

        while let Some(current) = to_visit.pop() {
            if let Some(neighbors) = self.nodes[current].connections.get(layer) {
                for &neighbor in neighbors {
                    if visited.contains(&neighbor) {
                        continue;
                    }
                    visited.insert(neighbor);
                    let dist = self.euclidean_distance(query, neighbor);
                    candidates.push((neighbor, dist));
                    to_visit.push(neighbor);
                }
            }
        }

        // Keep only best ef candidates (ascending for euclidean)
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        candidates.truncate(ef);
        candidates.into_iter().map(|(idx, _)| idx).collect()
    }

    fn distance(&self, query: &[f32], node_idx: usize) -> f32 {
        cosine_similarity(query, &self.nodes[node_idx].vector)
    }

    fn euclidean_distance(&self, query: &[f32], node_idx: usize) -> f32 {
        let node_vec = &self.nodes[node_idx].vector;
        let mut sum = 0.0f32;
        for i in 0..query.len().min(node_vec.len()) {
            let diff = query[i] - node_vec[i];
            sum += diff * diff;
        }
        sum.sqrt()
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

    #[cfg(feature = "std")]
    fn random_level(&self) -> usize {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let uniform: f64 = rng.gen_range(0.0..1.0);
        let uniform = uniform.max(f64::MIN_POSITIVE);
        let m_l = 1.0 / (self.m as f64).ln();
        let level = (-uniform.ln() * m_l).floor() as usize;
        level.min(self.max_layers.saturating_sub(1))
    }

    #[cfg(not(feature = "std"))]
    fn random_level(&self) -> usize {
        let mut level = 0;
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
                // Find the highest-level remaining node
                let mut new_ep = None;
                let mut max_level = 0;
                for (i, node) in self.nodes.iter().enumerate() {
                    let node_level = node.connections.len().saturating_sub(1);
                    if node_level >= max_level {
                        max_level = node_level;
                        new_ep = Some(i);
                    }
                }
                self.entry_point = new_ep;
            } else if let Some(ep) = self.entry_point {
                if ep > idx {
                    self.entry_point = Some(ep - 1);
                }
            }
        }
    }

    /// Serialize the index to a compact binary format
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Header
        bytes.extend_from_slice(&(self.nodes.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.max_layers as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.m as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.ef_construction as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.ef_search as u64).to_le_bytes());
        let ep = self.entry_point.map(|e| e as u64).unwrap_or(u64::MAX);
        bytes.extend_from_slice(&ep.to_le_bytes());
        bytes.extend_from_slice(&self.level_multiplier.to_le_bytes());

        // Nodes
        for node in &self.nodes {
            // ID
            let id_bytes = node.id.as_bytes();
            bytes.extend_from_slice(&(id_bytes.len() as u64).to_le_bytes());
            bytes.extend_from_slice(id_bytes);

            // Vector
            bytes.extend_from_slice(&(node.vector.len() as u64).to_le_bytes());
            for &v in &node.vector {
                bytes.extend_from_slice(&v.to_le_bytes());
            }

            // Connections
            bytes.extend_from_slice(&(node.connections.len() as u64).to_le_bytes());
            for layer in &node.connections {
                bytes.extend_from_slice(&(layer.len() as u64).to_le_bytes());
                for &conn in layer {
                    bytes.extend_from_slice(&(conn as u64).to_le_bytes());
                }
            }
        }

        bytes
    }

    /// Deserialize the index from a compact binary format
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut offset = 0;

        macro_rules! read_u64 {
            () => {{
                if bytes.len() < offset + 8 {
                    return Err("unexpected end of bytes");
                }
                let val = u64::from_le_bytes([
                    bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3],
                    bytes[offset + 4], bytes[offset + 5], bytes[offset + 6], bytes[offset + 7],
                ]);
                offset += 8;
                val
            }};
        }

        macro_rules! read_f32 {
            () => {{
                if bytes.len() < offset + 4 {
                    return Err("unexpected end of bytes");
                }
                let val = f32::from_le_bytes([
                    bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3],
                ]);
                offset += 4;
                val
            }};
        }

        let num_nodes = read_u64!() as usize;
        let max_layers = read_u64!() as usize;
        let m = read_u64!() as usize;
        let ef_construction = read_u64!() as usize;
        let ef_search = read_u64!() as usize;
        let entry_point_raw = read_u64!();
        let entry_point = if entry_point_raw == u64::MAX {
            None
        } else {
            Some(entry_point_raw as usize)
        };
        let level_multiplier = read_f32!();

        let mut nodes = Vec::with_capacity(num_nodes);
        let mut id_to_idx = BTreeMap::new();

        for i in 0..num_nodes {
            let id_len = read_u64!() as usize;
            if bytes.len() < offset + id_len {
                return Err("unexpected end of bytes");
            }
            let id = String::from_utf8(bytes[offset..offset + id_len].to_vec())
                .map_err(|_| "invalid utf8")?;
            offset += id_len;

            let vec_dim = read_u64!() as usize;
            if bytes.len() < offset + vec_dim * 4 {
                return Err("unexpected end of bytes");
            }
            let mut vector = Vec::with_capacity(vec_dim);
            for _ in 0..vec_dim {
                vector.push(read_f32!());
            }

            let num_layers = read_u64!() as usize;
            let mut connections = Vec::with_capacity(num_layers);
            for _ in 0..num_layers {
                let num_conns = read_u64!() as usize;
                if bytes.len() < offset + num_conns * 8 {
                    return Err("unexpected end of bytes");
                }
                let mut layer = Vec::with_capacity(num_conns);
                for _ in 0..num_conns {
                    layer.push(read_u64!() as usize);
                }
                connections.push(layer);
            }

            id_to_idx.insert(id.clone(), i);
            nodes.push(HnswNode {
                id,
                vector,
                connections,
            });
        }

        Ok(Self {
            nodes,
            id_to_idx,
            max_layers,
            m,
            ef_construction,
            ef_search,
            entry_point,
            level_multiplier,
        })
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

    #[test]
    #[cfg(feature = "std")]
    fn test_hnsw_random_level_distribution() {
        let mut index = HnswIndex::new(10, 16, 32);
        // Insert many nodes and check levels are not all the same
        for i in 0..500 {
            let vec = vec![i as f32; 10];
            index.insert(&format!("v{}", i), vec);
        }
        // Collect levels
        let levels: Vec<usize> = index
            .nodes
            .iter()
            .map(|n| n.connections.len().saturating_sub(1))
            .collect();
        let max_level = *levels.iter().max().unwrap_or(&0);
        assert!(
            max_level > 0,
            "All nodes have level 0, hierarchy is useless"
        );

        // Check that at least some nodes have level > 0
        let high_level_count = levels.iter().filter(|&&l| l > 0).count();
        assert!(high_level_count > 0, "No nodes have level > 0");
    }

    #[test]
    fn test_hnsw_serialization() {
        let mut index = HnswIndex::new(3, 8, 16);
        index.insert("a", vec![1.0, 0.0, 0.0]);
        index.insert("b", vec![0.0, 1.0, 0.0]);
        index.insert("c", vec![0.0, 0.0, 1.0]);

        let bytes = index.to_bytes();
        let restored = HnswIndex::from_bytes(&bytes).expect("deserialization failed");

        assert_eq!(restored.len(), index.len());

        let results = restored.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
    }
}
