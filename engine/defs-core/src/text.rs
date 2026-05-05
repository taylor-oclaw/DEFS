//! # Full-Text Index
//!
//! Inverted index for fast text search across particle dimensions.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Simple tokenizer: split on whitespace and punctuation, lowercase
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// Inverted index: term → [(particle_id, dimension_name, frequency)]
pub struct TextIndex {
    index: BTreeMap<String, Vec<TextIndexEntry>>,
    total_docs: usize,
}

#[derive(Clone, Debug)]
pub struct TextIndexEntry {
    pub particle_id: String,
    pub dimension: String,
    pub frequency: u32,
}

impl TextIndex {
    pub fn new() -> Self {
        Self {
            index: BTreeMap::new(),
            total_docs: 0,
        }
    }

    /// Index text from a particle dimension
    pub fn index(&mut self, particle_id: &str, dimension: &str, text: &str) {
        let tokens = tokenize(text);
        let mut term_freqs: BTreeMap<String, u32> = BTreeMap::new();

        for token in tokens {
            *term_freqs.entry(token).or_insert(0) += 1;
        }

        for (term, freq) in term_freqs {
            let entries = self.index.entry(term).or_insert_with(Vec::new);
            entries.push(TextIndexEntry {
                particle_id: String::from(particle_id),
                dimension: String::from(dimension),
                frequency: freq,
            });
        }

        self.total_docs += 1;
    }

    /// Remove all entries for a particle
    pub fn remove(&mut self, particle_id: &str) {
        for entries in self.index.values_mut() {
            entries.retain(|e| e.particle_id != particle_id);
        }
        self.index.retain(|_, entries| !entries.is_empty());
        self.total_docs = self.total_docs.saturating_sub(1);
    }

    /// Search for particles containing all terms
    pub fn search(&self, query: &str) -> Vec<(String, f32)> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        // Find particles that contain ALL query terms
        let mut candidate_scores: BTreeMap<String, (f32, u32)> = BTreeMap::new();

        for term in &query_terms {
            if let Some(entries) = self.index.get(term) {
                for entry in entries {
                    let (score, count) = candidate_scores
                        .entry(entry.particle_id.clone())
                        .or_insert((0.0, 0));
                    *score += entry.frequency as f32;
                    *count += 1;
                }
            } else {
                // Term not found → no results
                return Vec::new();
            }
        }

        // Only keep particles that matched all terms
        let mut results: Vec<(String, f32)> = candidate_scores
            .into_iter()
            .filter(|(_, (_, count))| *count as usize == query_terms.len())
            .map(|(id, (score, _))| (id, score / query_terms.len() as f32))
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal));
        results
    }

    /// Search for particles containing ANY term
    pub fn search_any(&self, query: &str) -> Vec<(String, f32)> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let mut candidate_scores: BTreeMap<String, f32> = BTreeMap::new();

        for term in &query_terms {
            if let Some(entries) = self.index.get(term) {
                for entry in entries {
                    let score = entry.frequency as f32;
                    *candidate_scores
                        .entry(entry.particle_id.clone())
                        .or_insert(0.0) += score;
                }
            }
        }

        let mut results: Vec<(String, f32)> = candidate_scores.into_iter().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(core::cmp::Ordering::Equal));
        results
    }

    pub fn term_count(&self) -> usize {
        self.index.len()
    }

    pub fn total_docs(&self) -> usize {
        self.total_docs
    }
}

/// Detect content type from byte magic numbers
pub fn detect_content_type(data: &[u8]) -> &'static str {
    if data.len() < 4 {
        return "application/octet-stream";
    }

    match &data[0..4] {
        b"%PDF" => "application/pdf",
        b"PK\x03\x04" => "application/zip", // docx, xlsx, etc.
        b"\x89PNG" => "image/png",
        b"\xFF\xD8\xFF\xE0" | b"\xFF\xD8\xFF\xE1" | b"\xFF\xD8\xFF\xDB" => "image/jpeg",
        b"GIF8" => "image/gif",
        b"\x1F\x8B" => "application/gzip",
        _ => {
            // Check for text
            if data
                .iter()
                .all(|&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
            {
                // Try to detect specific text types
                let text = String::from_utf8_lossy(data);
                if text.starts_with("---") || text.contains("title:") {
                    "text/yaml"
                } else if text.starts_with("#") || text.starts_with("//") || text.starts_with("/*")
                {
                    "text/plain" // could be code
                } else if text.starts_with("<?xml") || text.starts_with("<!") {
                    "text/xml"
                } else if text.starts_with("{") || text.starts_with("[") {
                    "application/json"
                } else {
                    "text/plain"
                }
            } else {
                "application/octet-stream"
            }
        }
    }
}

/// Extract plain text from common formats (Phase 2: basic)
pub fn extract_text(data: &[u8], content_type: &str) -> Option<String> {
    match content_type {
        "text/plain" | "text/markdown" | "text/yaml" | "text/xml" | "application/json" => {
            String::from_utf8(data.to_vec()).ok()
        }
        _ => {
            // For binary formats, return first N bytes as preview
            let preview_len = data.len().min(256);
            let preview = &data[..preview_len];
            if preview
                .iter()
                .all(|&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
            {
                String::from_utf8(preview.to_vec()).ok()
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let text = "Hello, world! This is a test.";
        let tokens = tokenize(text);
        assert_eq!(tokens, vec!["hello", "world", "this", "is", "a", "test"]);
    }

    #[test]
    fn test_text_index_search() {
        let mut index = TextIndex::new();
        index.index("p1", "content", "hello world");
        index.index("p2", "content", "hello rust");
        index.index("p3", "content", "goodbye world");

        let results = index.search("hello world");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "p1");

        let results_any = index.search_any("hello world");
        assert_eq!(results_any.len(), 3);
    }

    #[test]
    fn test_detect_content_type() {
        assert_eq!(detect_content_type(b"%PDF-1.4"), "application/pdf");
        assert_eq!(detect_content_type(b"\x89PNG\r\n\x1a\n"), "image/png");
        assert_eq!(detect_content_type(b"hello world"), "text/plain");
        assert_eq!(
            detect_content_type(b"{\"key\":\"value\"}"),
            "application/json"
        );
    }
}
