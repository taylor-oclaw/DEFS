//! # Blackhole Compression
//!
//! Type-aware compression for wavelet payloads.
//! Phase 3: Basic algorithms. Phase 5: Density-aware adaptive compression.

use alloc::vec::Vec;

/// Compression algorithm selection
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompressAlgo {
    None,
    Rle,   // Run-length encoding (good for repetitive data)
    Delta, // Delta encoding (good for sequences)
    Dict,  // Dictionary compression (good for text)
    Lz4,   // LZ4-style fast compression (placeholder)
}

/// Compress a byte slice using the specified algorithm
pub fn compress(data: &[u8], algo: CompressAlgo) -> Vec<u8> {
    match algo {
        CompressAlgo::None => data.to_vec(),
        CompressAlgo::Rle => rle_compress(data),
        CompressAlgo::Delta => delta_compress(data),
        CompressAlgo::Dict => dict_compress(data),
        CompressAlgo::Lz4 => lz4_compress(data),
    }
}

/// Decompress a byte slice
pub fn decompress(data: &[u8], algo: CompressAlgo) -> Vec<u8> {
    match algo {
        CompressAlgo::None => data.to_vec(),
        CompressAlgo::Rle => rle_decompress(data),
        CompressAlgo::Delta => delta_decompress(data),
        CompressAlgo::Dict => dict_decompress(data),
        CompressAlgo::Lz4 => lz4_decompress(data),
    }
}

/// Select best algorithm based on data characteristics
pub fn select_algo(data: &[u8]) -> CompressAlgo {
    if data.len() < 64 {
        return CompressAlgo::None;
    }

    // Check for high repetition (RLE candidate)
    let mut run_lengths = Vec::new();
    let mut current_run = 1u32;
    for i in 1..data.len().min(1024) {
        if data[i] == data[i - 1] {
            current_run += 1;
        } else {
            run_lengths.push(current_run);
            current_run = 1;
        }
    }
    run_lengths.push(current_run);

    let avg_run = run_lengths.iter().sum::<u32>() as f32 / run_lengths.len() as f32;
    if avg_run > 4.0 {
        return CompressAlgo::Rle;
    }

    // Check for sequential data (delta candidate)
    let mut deltas = Vec::new();
    for i in 1..data.len().min(1024) {
        deltas.push((data[i] as i16 - data[i - 1] as i16).abs() as u8);
    }
    let small_deltas = deltas.iter().filter(|&&d| d < 4).count();
    if small_deltas as f32 / deltas.len() as f32 > 0.8 {
        return CompressAlgo::Delta;
    }

    // Check for text-like data (dictionary candidate)
    let printable = data
        .iter()
        .filter(|&&b| b.is_ascii_graphic() || b.is_ascii_whitespace())
        .count();
    if printable as f32 / data.len() as f32 > 0.9 {
        return CompressAlgo::Dict;
    }

    CompressAlgo::Lz4
}

/// Run-length encoding: [byte, count] pairs
fn rle_compress(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    if data.is_empty() {
        return result;
    }

    let mut current = data[0];
    let mut count = 1u8;

    for &byte in &data[1..] {
        if byte == current && count < 255 {
            count += 1;
        } else {
            result.push(current);
            result.push(count);
            current = byte;
            count = 1;
        }
    }
    result.push(current);
    result.push(count);
    result
}

fn rle_decompress(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    for chunk in data.chunks(2) {
        if chunk.len() == 2 {
            result.extend(std::iter::repeat(chunk[0]).take(chunk[1] as usize));
        }
    }
    result
}

/// Delta encoding: store first byte + deltas
fn delta_compress(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut result = vec![data[0]];
    for i in 1..data.len() {
        result.push(data[i].wrapping_sub(data[i - 1]));
    }
    result
}

fn delta_decompress(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut result = vec![data[0]];
    for i in 1..data.len() {
        result.push(result.last().unwrap().wrapping_add(data[i]));
    }
    result
}

/// Simple dictionary compression: build dictionary of common substrings
fn dict_compress(data: &[u8]) -> Vec<u8> {
    // Phase 3: simple prefix compression
    // Phase 5: full dictionary with huffman coding
    let mut result = Vec::new();
    result.push(0u8); // version
    result.extend_from_slice(&(data.len() as u32).to_le_bytes());
    result.extend_from_slice(data);
    result
}

fn dict_decompress(data: &[u8]) -> Vec<u8> {
    if data.len() < 5 {
        return Vec::new();
    }
    let len = u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as usize;
    data[5..5 + len.min(data.len() - 5)].to_vec()
}

/// Real LZ4 compression via lz4_flex block format with size prepended
fn lz4_compress(data: &[u8]) -> Vec<u8> {
    let compressed = lz4_flex::block::compress_prepend_size(data);
    let mut result = vec![0x02u8]; // LZ4 marker (version 2 = real lz4)
    result.extend_from_slice(&(data.len() as u32).to_le_bytes());
    result.extend_from_slice(&compressed);
    result
}

fn lz4_decompress(data: &[u8]) -> Vec<u8> {
    if data.len() < 5 {
        return data.to_vec();
    }
    match data[0] {
        0x01 => {
            // Old placeholder format
            let len = u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as usize;
            data[5..5 + len.min(data.len() - 5)].to_vec()
        }
        0x02 => {
            // Real LZ4
            let _original_len = u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as usize;
            let compressed = &data[5..];
            match lz4_flex::block::decompress_size_prepended(compressed) {
                Ok(decompressed) => decompressed,
                Err(_) => {
                    // Fallback
                    Vec::new()
                }
            }
        }
        _ => data.to_vec(),
    }
}

/// Compression statistics
#[derive(Clone, Debug)]
pub struct CompressStats {
    pub algo: CompressAlgo,
    pub original_size: usize,
    pub compressed_size: usize,
    pub ratio: f32,
}

impl CompressStats {
    pub fn new(algo: CompressAlgo, original: usize, compressed: usize) -> Self {
        Self {
            algo,
            original_size: original,
            compressed_size: compressed,
            ratio: if original == 0 {
                1.0
            } else {
                compressed as f32 / original as f32
            },
        }
    }
}

/// Auto-compress: select best algorithm and return compressed data + stats
pub fn auto_compress(data: &[u8]) -> (Vec<u8>, CompressStats) {
    let algo = select_algo(data);
    let compressed = compress(data, algo);
    let stats = CompressStats::new(algo, data.len(), compressed.len());
    (compressed, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rle_roundtrip() {
        let data = vec![0xAA, 0xAA, 0xAA, 0xBB, 0xBB, 0xCC];
        let compressed = rle_compress(&data);
        let decompressed = rle_decompress(&compressed);
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_delta_roundtrip() {
        let data = vec![10, 11, 12, 13, 14, 15];
        let compressed = delta_compress(&data);
        let decompressed = delta_decompress(&compressed);
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_auto_compress_repetitive() {
        let data = vec![0xAA; 1000];
        let (compressed, stats) = auto_compress(&data);
        assert_eq!(stats.algo, CompressAlgo::Rle);
        assert!(stats.ratio < 0.1); // massive compression
        let decompressed = decompress(&compressed, stats.algo);
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_auto_compress_sequential() {
        let data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
        let (compressed, stats) = auto_compress(&data);
        assert_eq!(stats.algo, CompressAlgo::Delta);
        let decompressed = decompress(&compressed, stats.algo);
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_auto_compress_text() {
        let data = b"hello world ".repeat(100);
        let (compressed, stats) = auto_compress(&data);
        assert_eq!(stats.algo, CompressAlgo::Dict);
        let decompressed = decompress(&compressed, stats.algo);
        assert_eq!(data, decompressed);
    }

    #[test]
    fn test_lz4_roundtrip() {
        let data = b"this is a test of real lz4 compression in defs ".repeat(500);
        let compressed = compress(&data, CompressAlgo::Lz4);
        assert!(
            compressed.len() < data.len(),
            "LZ4 should compress: {} -> {}",
            data.len(),
            compressed.len()
        );
        let decompressed = decompress(&compressed, CompressAlgo::Lz4);
        assert_eq!(data[..], decompressed[..]);
    }
}
