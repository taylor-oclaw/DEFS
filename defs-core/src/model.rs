//! # Model-Aware Storage
//!
//! Layer-addressable AI model storage (Patent #2).
//! Weight-delta deduplication for fine-tunes (Patent #3).
//! KV cache persistence (Patent #4).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::particle::{GravityKind, Particle, ParticleId, Wavelet};
use crate::store::StoreError;

/// Types of model layers
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerType {
    TokenEmbedding,
    AttentionQ,
    AttentionK,
    AttentionV,
    AttentionOut,
    FeedForwardUp,
    FeedForwardDown,
    LayerNorm,
    OutputNorm,
    OutputHead,
    Custom,
}

/// Model weight data type
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WeightType {
    Float32,
    Float16,
    BFloat16,
    Int8,
    Int4,
}

/// A single layer in a model
#[derive(Clone, Debug)]
pub struct ModelLayer {
    pub name: String,
    pub layer_type: LayerType,
    pub shape: Vec<u64>,
    pub weight_type: WeightType,
    pub data: Vec<u8>,
}

/// Model metadata
#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub model_id: String,
    pub architecture: String,
    pub base_model: Option<String>,
    pub total_params: u64,
    pub total_layers: usize,
    pub quantization: Option<String>,
}

/// Model storage using DEFS particles
///
/// Each model is a particle with:
/// - metadata dimensions (model_id, architecture, etc.)
/// - layer dimensions (one per layer, containing compressed weight data)
/// - gravity bonds to base model (for fine-tunes)
pub struct ModelStore {
    models: BTreeMap<String, Particle>,
    _next_id: u64,
}

impl ModelStore {
    pub fn new() -> Self {
        Self {
            models: BTreeMap::new(),
            _next_id: 1,
        }
    }

    /// Store a base model as a particle with layer dimensions
    pub fn store_base(
        &mut self,
        info: ModelInfo,
        layers: Vec<ModelLayer>,
    ) -> Result<ParticleId, StoreError> {
        let id = ParticleId::from_content(format!("model:{}", info.model_id).as_bytes());
        let mut particle = Particle::new(id.clone());

        particle.set_dimension("_type", Wavelet::from_string("ai_model"));
        particle.set_dimension("model_id", Wavelet::from_string(&info.model_id));
        particle.set_dimension("architecture", Wavelet::from_string(&info.architecture));
        particle.set_dimension(
            "total_params",
            Wavelet::from_int64(info.total_params as i64),
        );
        particle.set_dimension(
            "total_layers",
            Wavelet::from_int64(info.total_layers as i64),
        );
        particle.set_dimension("layer_count", Wavelet::from_int64(layers.len() as i64));

        if let Some(q) = info.quantization {
            particle.set_dimension("quantization", Wavelet::from_string(&q));
        }

        // Store each layer as a dimension
        for layer in &layers {
            let dim_name = format!("layer:{}", layer.name);
            particle.set_dimension(&dim_name, Wavelet::from_binary(&layer.data));
        }

        self.models.insert(info.model_id.clone(), particle.clone());
        Ok(id)
    }

    /// Store a fine-tuned model (only changed layers)
    pub fn store_finetune(
        &mut self,
        info: ModelInfo,
        base_model_id: &str,
        changed_layers: Vec<ModelLayer>,
    ) -> Result<ParticleId, StoreError> {
        let id = ParticleId::from_content(format!("model:{}:finetune", info.model_id).as_bytes());
        let mut particle = Particle::new(id.clone());

        particle.set_dimension("_type", Wavelet::from_string("ai_model_finetune"));
        particle.set_dimension("model_id", Wavelet::from_string(&info.model_id));
        particle.set_dimension("architecture", Wavelet::from_string(&info.architecture));
        particle.set_dimension("base_model", Wavelet::from_string(base_model_id));
        particle.set_dimension(
            "changed_layers",
            Wavelet::from_int64(changed_layers.len() as i64),
        );

        // Only store changed layers
        for layer in &changed_layers {
            let dim_name = format!("layer:{}", layer.name);
            particle.set_dimension(&dim_name, Wavelet::from_binary(&layer.data));
        }

        // Link to base model
        if let Some(base) = self.models.get(base_model_id) {
            particle.add_bond(base.id.clone(), GravityKind::DerivedFrom, 1.0);
        }

        self.models.insert(info.model_id.clone(), particle.clone());
        Ok(id)
    }

    /// Load a specific layer from a model
    pub fn load_layer(&self, model_id: &str, layer_name: &str) -> Result<Vec<u8>, StoreError> {
        let particle = self.models.get(model_id).ok_or(StoreError::NotFound)?;
        let dim_name = format!("layer:{}", layer_name);

        // Try to find layer in this particle
        if let Some(wavelet) = particle.dimension(&dim_name) {
            if let Some(data) = wavelet.as_binary() {
                return Ok(data.to_vec());
            }
        }

        // For fine-tunes: if layer not found here, load from base model
        if let Some(base_id) = particle.dimension("base_model").and_then(|w| w.as_str()) {
            if let Some(base) = self.models.get(base_id) {
                if let Some(wavelet) = base.dimension(&dim_name) {
                    if let Some(data) = wavelet.as_binary() {
                        return Ok(data.to_vec());
                    }
                }
            }
        }

        Err(StoreError::NotFound)
    }

    /// Store KV cache for a session
    pub fn store_kv_cache(
        &mut self,
        session_id: &str,
        model_id: &str,
        layer_caches: Vec<(String, Vec<u8>)>,
    ) -> Result<ParticleId, StoreError> {
        let id =
            ParticleId::from_content(format!("kv_cache:{}:{}", model_id, session_id).as_bytes());
        let mut particle = Particle::new(id.clone());

        particle.set_dimension("_type", Wavelet::from_string("kv_cache"));
        particle.set_dimension("session_id", Wavelet::from_string(session_id));
        particle.set_dimension("model_id", Wavelet::from_string(model_id));
        particle.set_dimension(
            "layer_count",
            Wavelet::from_int64(layer_caches.len() as i64),
        );

        for (layer_name, data) in layer_caches {
            let dim_name = format!("kv:{}", layer_name);
            particle.set_dimension(&dim_name, Wavelet::from_binary(&data));
        }

        // Link to model
        if let Some(model) = self.models.get(model_id) {
            particle.add_bond(model.id.clone(), GravityKind::ComputedBy, 1.0);
        }

        let key = format!("kv:{}:{}", model_id, session_id);
        self.models.insert(key, particle);
        Ok(id)
    }

    /// Load KV cache for a session
    pub fn load_kv_cache(
        &self,
        session_id: &str,
        model_id: &str,
    ) -> Result<Vec<(String, Vec<u8>)>, StoreError> {
        let key = format!("kv:{}:{}", model_id, session_id);
        let particle = self.models.get(&key).ok_or(StoreError::NotFound)?;

        let mut caches = Vec::new();
        for (dim_name, wavelet) in &particle.dimensions {
            if dim_name.starts_with("kv:") {
                if let Some(data) = wavelet.as_binary() {
                    caches.push((dim_name[3..].to_string(), data.to_vec()));
                }
            }
        }

        Ok(caches)
    }

    pub fn get_model(&self, model_id: &str) -> Option<&Particle> {
        self.models.get(model_id)
    }

    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// Calculate storage savings from delta encoding
    pub fn delta_savings(&self) -> (u64, u64) {
        let mut total_full = 0u64;
        let mut total_stored = 0u64;

        for particle in self.models.values() {
            let mut stored = 0u64;
            for (name, wavelet) in &particle.dimensions {
                if name.starts_with("layer:") || name.starts_with("kv:") {
                    stored += wavelet.payload.len() as u64;
                }
            }
            total_stored += stored;

            // If fine-tune, add base model size to "full" cost
            if let Some(base_id) = particle.dimension("base_model").and_then(|w| w.as_str()) {
                if let Some(base) = self.models.get(base_id) {
                    let mut base_size = 0u64;
                    for (name, wavelet) in &base.dimensions {
                        if name.starts_with("layer:") {
                            base_size += wavelet.payload.len() as u64;
                        }
                    }
                    total_full += base_size;
                }
            } else {
                total_full += stored;
            }
        }

        (total_full, total_stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_and_load_base_model() {
        let mut store = ModelStore::new();
        let info = ModelInfo {
            model_id: String::from("llama-3-8b"),
            architecture: String::from("llama"),
            base_model: None,
            total_params: 8_000_000_000,
            total_layers: 32,
            quantization: Some(String::from("Q4_K_M")),
        };

        let layers = vec![
            ModelLayer {
                name: String::from("embeddings"),
                layer_type: LayerType::TokenEmbedding,
                shape: vec![32000, 4096],
                weight_type: WeightType::Float16,
                data: vec![0u8; 100],
            },
            ModelLayer {
                name: String::from("layer_0_attn_q"),
                layer_type: LayerType::AttentionQ,
                shape: vec![4096, 4096],
                weight_type: WeightType::Float16,
                data: vec![1u8; 100],
            },
        ];

        let id = store.store_base(info, layers).unwrap();
        assert!(!id.is_null());

        let loaded = store.load_layer("llama-3-8b", "embeddings").unwrap();
        assert_eq!(loaded, vec![0u8; 100]);

        let loaded = store.load_layer("llama-3-8b", "layer_0_attn_q").unwrap();
        assert_eq!(loaded, vec![1u8; 100]);
    }

    #[test]
    fn test_finetune_delta_storage() {
        let mut store = ModelStore::new();

        // Store base model
        let base_info = ModelInfo {
            model_id: String::from("llama-3-8b"),
            architecture: String::from("llama"),
            base_model: None,
            total_params: 8_000_000_000,
            total_layers: 32,
            quantization: None,
        };
        let base_layers = vec![
            ModelLayer {
                name: String::from("layer_0"),
                layer_type: LayerType::AttentionQ,
                shape: vec![4096, 4096],
                weight_type: WeightType::Float16,
                data: vec![0u8; 1000],
            },
            ModelLayer {
                name: String::from("layer_1"),
                layer_type: LayerType::AttentionK,
                shape: vec![4096, 4096],
                weight_type: WeightType::Float16,
                data: vec![1u8; 1000],
            },
        ];
        store.store_base(base_info, base_layers).unwrap();

        // Store fine-tune (only layer_0 changed)
        let finetune_info = ModelInfo {
            model_id: String::from("llama-3-8b-legal"),
            architecture: String::from("llama"),
            base_model: Some(String::from("llama-3-8b")),
            total_params: 8_000_000_000,
            total_layers: 32,
            quantization: None,
        };
        let changed_layers = vec![ModelLayer {
            name: String::from("layer_0"),
            layer_type: LayerType::AttentionQ,
            shape: vec![4096, 4096],
            weight_type: WeightType::Float16,
            data: vec![2u8; 100], // smaller delta
        }];
        store
            .store_finetune(finetune_info, "llama-3-8b", changed_layers)
            .unwrap();

        // Load layer_0 from fine-tune → should get changed version
        let loaded = store.load_layer("llama-3-8b-legal", "layer_0").unwrap();
        assert_eq!(loaded, vec![2u8; 100]);

        // Load layer_1 from fine-tune → should fall back to base model
        let loaded = store.load_layer("llama-3-8b-legal", "layer_1").unwrap();
        assert_eq!(loaded, vec![1u8; 1000]);

        // Check savings
        let (full, stored) = store.delta_savings();
        assert!(full > stored); // Delta saves space
    }

    #[test]
    fn test_kv_cache_persistence() {
        let mut store = ModelStore::new();

        // Store base model first (needed for KV cache linking)
        let base_info = ModelInfo {
            model_id: String::from("test-model"),
            architecture: String::from("test"),
            base_model: None,
            total_params: 100,
            total_layers: 2,
            quantization: None,
        };
        store.store_base(base_info, vec![]).unwrap();

        // Store KV cache
        let caches = vec![
            (String::from("layer_0"), vec![0u8; 100]),
            (String::from("layer_1"), vec![1u8; 100]),
        ];
        store
            .store_kv_cache("session_123", "test-model", caches)
            .unwrap();

        // Load KV cache
        let loaded = store.load_kv_cache("session_123", "test-model").unwrap();
        assert_eq!(loaded.len(), 2);
    }
}
