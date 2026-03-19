use alloc::string::String;
use alloc::vec::Vec;
use alloc::string::String;

pub struct ModelLayer {
    pub name: String,
    pub layer_type: LayerType,
    pub offset: u64,
    pub size: u64,
    pub shape: Vec<u32>,
    pub dtype: DataType
}

pub enum LayerType {
    Embedding,
    Attention,
    FeedForward,
    Normalization,
    Output,
    Custom
}

pub enum DataType {
    Float32,
    Float16,
    BFloat16,
    Int8,
    Int4
}

pub struct ModelMetadata {
    pub name: String,
    pub architecture: String,
    pub format: ModelFormat,
    pub total_params: u64,
    pub layers: Vec<ModelLayer>,
    pub base_model_hash: Option<[u8; 32]>,
    pub is_finetuned: bool,
    pub delta_size: u64,
    pub quantization: Option<String>
}

pub enum ModelFormat {
    GGUF,
    ONNX,
    SafeTensors,
    PyTorch,
    Custom
}

pub struct ModelStore {
    pub models: Vec<ModelMetadata>
}

impl ModelStore {
    pub fn new() -> Self {
        Self { models: Vec::new() }
    }

    pub fn register(&mut self, model: ModelMetadata) {
        self.models.push(model);
    }

    pub fn find_by_name(&self, name: &str) -> Option<&ModelMetadata> {
        self.models.iter().find(|m| m.name == name)
    }

    pub fn get_layer(&self, model_name: &str, layer_name: &str) -> Option<&ModelLayer> {
        self.find_by_name(model_name).and_then(|m| m.layers.iter().find(|l| l.name == layer_name))
    }

    pub fn delta_savings(&self) -> u64 {
        self.models.iter().filter(|m| m.is_finetuned).map(|m| m.total_params * 4 - m.delta_size).sum()
    }

    pub fn total_storage(&self) -> u64 {
        self.models.iter().map(|m| m.layers.iter().map(|l| l.size).sum::<u64>()).sum()
    }
}
