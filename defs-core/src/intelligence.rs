//! # AI Metadata Extraction Pipeline
//!
//! Analyzes particle content and generates semantic metadata:
//! - Content type detection
//! - Extracted text
//! - Auto-generated tags
//! - Quality scoring
//! - Language detection

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::particle::{Particle, Wavelet};
use crate::text::{detect_content_type, extract_text, tokenize};

/// Generated intelligence for a particle
#[derive(Clone, Debug)]
pub struct ParticleIntelligence {
    pub content_type: String,
    pub extracted_text: Option<String>,
    pub tags: Vec<String>,
    pub language: Option<String>,
    pub quality: f32,
    pub summary: Option<String>,
}

/// AI pipeline for analyzing particles
pub struct IntelligenceEngine;

impl IntelligenceEngine {
    pub fn new() -> Self {
        Self
    }

    /// Analyze a particle and generate intelligence
    pub fn analyze(&self, particle: &Particle) -> ParticleIntelligence {
        let content = particle.content();
        let content_type = particle
            .dimension("content_type")
            .and_then(|w| w.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                content
                    .map(|c| String::from(detect_content_type(&c.payload)))
                    .unwrap_or_else(|| String::from("application/octet-stream"))
            });

        let extracted_text = content.and_then(|c| extract_text(&c.payload, &content_type));

        let tags = if let Some(ref text) = extracted_text {
            Self::generate_tags(text, &content_type)
        } else {
            Vec::new()
        };

        let language = extracted_text
            .as_ref()
            .and_then(|text| Self::detect_language(text));

        let quality = Self::score_quality(particle, &extracted_text, &tags);

        let summary = extracted_text
            .as_ref()
            .map(|text| Self::generate_summary(text));

        ParticleIntelligence {
            content_type,
            extracted_text,
            tags,
            language,
            quality,
            summary,
        }
    }

    /// Apply intelligence as dimensions on a particle
    pub fn enrich(&self, particle: &mut Particle) {
        let intel = self.analyze(particle);

        particle.set_dimension("_detected_type", Wavelet::from_string(&intel.content_type));

        if let Some(text) = intel.extracted_text {
            particle.set_dimension("_extracted_text", Wavelet::from_string(&text));
        }

        if !intel.tags.is_empty() {
            particle.set_dimension("_auto_tags", Wavelet::from_string(&intel.tags.join(", ")));
        }

        if let Some(lang) = intel.language {
            particle.set_dimension("_language", Wavelet::from_string(&lang));
        }

        particle.set_dimension("_quality", Wavelet::from_float64(intel.quality as f64));

        if let Some(summary) = intel.summary {
            particle.set_dimension("_summary", Wavelet::from_string(&summary));
        }
    }

    /// Generate tags from text content
    fn generate_tags(text: &str, content_type: &str) -> Vec<String> {
        let mut tags = BTreeSet::new();
        let text_lower = text.to_lowercase();

        // Content-type based tags
        match content_type {
            "application/pdf" => {
                tags.insert(String::from("document"));
                tags.insert(String::from("pdf"));
            }
            "text/plain" => {
                tags.insert(String::from("text"));
            }
            "text/markdown" => {
                tags.insert(String::from("markdown"));
                tags.insert(String::from("documentation"));
            }
            "application/json" => {
                tags.insert(String::from("data"));
                tags.insert(String::from("json"));
            }
            "image/png" | "image/jpeg" | "image/gif" => {
                tags.insert(String::from("image"));
            }
            _ => {}
        }

        // Domain-based tags
        if text_lower.contains("finance")
            || text_lower.contains("budget")
            || text_lower.contains("revenue")
        {
            tags.insert(String::from("finance"));
        }
        if text_lower.contains("medical")
            || text_lower.contains("health")
            || text_lower.contains("patient")
        {
            tags.insert(String::from("medical"));
        }
        if text_lower.contains("code")
            || text_lower.contains("function")
            || text_lower.contains("api")
        {
            tags.insert(String::from("code"));
        }
        if text_lower.contains("meeting")
            || text_lower.contains("agenda")
            || text_lower.contains("minutes")
        {
            tags.insert(String::from("meeting"));
        }
        if text_lower.contains("report") || text_lower.contains("analysis") {
            tags.insert(String::from("report"));
        }
        if text_lower.contains("contract")
            || text_lower.contains("agreement")
            || text_lower.contains("legal")
        {
            tags.insert(String::from("legal"));
        }

        // Top frequent words as tags (excluding stop words)
        let tokens = tokenize(text);
        let stop_words: BTreeSet<&str> = [
            "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has",
            "had", "do", "does", "did", "will", "would", "could", "should", "of", "for", "with",
            "about", "against", "between", "into", "through", "during", "and", "but", "or", "yet",
            "so", "if", "then", "than", "when", "where", "why", "how", "this", "that", "these",
            "those", "i", "you", "he", "she", "it", "we", "they", "to", "from", "in", "on", "at",
            "by", "as", "up", "down", "out", "off", "over", "under",
        ]
        .iter()
        .cloned()
        .collect();

        let mut word_freqs: BTreeMap<String, u32> = BTreeMap::new();
        for token in tokens {
            if token.len() > 3 && !stop_words.contains(token.as_str()) {
                *word_freqs.entry(token).or_insert(0) += 1;
            }
        }

        let mut freq_vec: Vec<(String, u32)> = word_freqs.into_iter().collect();
        freq_vec.sort_by(|a, b| b.1.cmp(&a.1));

        for (word, _) in freq_vec.into_iter().take(3) {
            tags.insert(word);
        }

        tags.into_iter().collect()
    }

    /// Simple language detection
    fn detect_language(text: &str) -> Option<String> {
        // Very basic heuristic
        let text_lower = text.to_lowercase();
        if text_lower.contains(" the ")
            || text_lower.contains(" and ")
            || text_lower.contains(" of ")
        {
            Some(String::from("en"))
        } else if text_lower.contains(" el ")
            || text_lower.contains(" la ")
            || text_lower.contains(" de ")
        {
            Some(String::from("es"))
        } else if text_lower.contains(" le ")
            || text_lower.contains(" et ")
            || text_lower.contains(" de ")
        {
            Some(String::from("fr"))
        } else if text_lower.contains(" der ")
            || text_lower.contains(" die ")
            || text_lower.contains(" und ")
        {
            Some(String::from("de"))
        } else {
            None
        }
    }

    /// Score particle quality (0.0 - 1.0)
    fn score_quality(particle: &Particle, extracted_text: &Option<String>, tags: &[String]) -> f32 {
        let mut score = 0.5f32;

        // Has name dimension
        if particle.name().is_some() {
            score += 0.1;
        }

        // Has content
        if particle.content().is_some() {
            score += 0.1;
        }

        // Has extracted text
        if extracted_text.is_some() {
            score += 0.1;
        }

        // Has tags
        if !tags.is_empty() {
            score += 0.1 * (tags.len() as f32 / 5.0).min(1.0);
        }

        // Has gravity bonds (connected to other particles)
        if !particle.gravity.is_empty() {
            score += 0.1;
        }

        score.min(1.0)
    }

    /// Generate a simple summary
    fn generate_summary(text: &str) -> String {
        let sentences: Vec<&str> = text.split('.').filter(|s| !s.trim().is_empty()).collect();
        if sentences.is_empty() {
            return String::from("(no text)");
        }
        let summary = sentences[0].trim();
        if summary.len() > 200 {
            format!("{}...", &summary[..200])
        } else {
            String::from(summary)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::ParticleId;

    #[test]
    fn test_analyze_pdf() {
        let mut p = Particle::new(ParticleId::from_content(b"doc"));
        p.set_dimension(
            "content",
            Wavelet::from_binary(b"%PDF-1.4\nThis is a finance report about revenue and budget."),
        );
        p.set_dimension("name", Wavelet::from_string("report.pdf"));

        let engine = IntelligenceEngine::new();
        let intel = engine.analyze(&p);

        assert_eq!(intel.content_type, "application/pdf");
        assert!(intel.extracted_text.is_some());
        assert!(!intel.tags.is_empty());
        assert!(intel.tags.contains(&String::from("finance")));
        assert!(intel.tags.contains(&String::from("document")));
        assert!(intel.quality > 0.5);
    }

    #[test]
    fn test_enrich_particle() {
        let mut p = Particle::new(ParticleId::from_content(b"doc"));
        p.set_dimension(
            "content",
            Wavelet::from_binary(b"hello world this is a test document"),
        );
        p.set_dimension("name", Wavelet::from_string("test.txt"));

        let engine = IntelligenceEngine::new();
        engine.enrich(&mut p);

        assert!(p.dimension("_detected_type").is_some());
        assert!(p.dimension("_extracted_text").is_some());
        assert!(p.dimension("_auto_tags").is_some());
        assert!(p.dimension("_quality").is_some());
    }
}
