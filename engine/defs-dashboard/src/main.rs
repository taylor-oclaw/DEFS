//! DEFS Web Dashboard
//!
//! A lightweight HTTP server providing real-time visualization of a DEFS volume.
//! Run: `defs-dashboard --volume /path/to/my.defs --port 8765`

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;

use defs_core::persist::PersistentStore;
use defs_core::store::SearchQuery;
use serde::Serialize;

const INDEX_HTML: &str = include_str!("../static/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const STYLE_CSS: &str = include_str!("../static/style.css");

#[derive(Serialize)]
struct VolumeStats {
    label: String,
    version: String,
    size_mb: u64,
    used_bytes: u64,
    free_bytes: u64,
    particle_count: usize,
    singularity_count: usize,
    features: Vec<String>,
    dimensions: Vec<(String, usize)>,
    bond_kinds: Vec<(String, usize)>,
}

#[derive(Serialize)]
struct ParticleSummary {
    id: String,
    name: Option<String>,
    content_type: Option<String>,
    created_at: u64,
    modified_at: u64,
    dimension_count: usize,
    bond_count: usize,
    incoming_count: usize,
}

#[derive(Serialize)]
struct GraphNode {
    id: String,
    name: String,
    group: String,
}

#[derive(Serialize)]
struct GraphLink {
    source: String,
    target: String,
    kind: String,
    strength: f64,
}

#[derive(Serialize)]
struct GraphData {
    nodes: Vec<GraphNode>,
    links: Vec<GraphLink>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut volume_path: Option<PathBuf> = None;
    let mut port = 8765u16;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--volume" | "-v" => {
                i += 1;
                if i < args.len() {
                    volume_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--port" | "-p" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().unwrap_or(8765);
                }
            }
            "--help" | "-h" => {
                println!("DEFS Dashboard v{}", env!("CARGO_PKG_VERSION"));
                println!("Usage: defs-dashboard --volume <path> [--port <port>]");
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    let volume_path = volume_path.expect("--volume required. Use --help for usage.");

    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║          DEFS Dashboard v{}                        ║", env!("CARGO_PKG_VERSION"));
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Volume: {:47} ║", volume_path.display());
    println!("║  URL:    http://localhost:{:<39} ║", port);
    println!("╚══════════════════════════════════════════════════════════╝");

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).expect("Failed to bind port");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let path = volume_path.clone();
                thread::spawn(move || handle_request(stream, &path));
            }
            Err(e) => eprintln!("Connection error: {}", e),
        }
    }
}

fn handle_request(mut stream: TcpStream, volume_path: &PathBuf) {
    let mut buffer = [0u8; 4096];
    let n = match stream.read(&mut buffer) {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let request = String::from_utf8_lossy(&buffer[..n]);
    let line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    let method = parts[0];
    let path = parts[1];

    if method != "GET" {
        send_405(&mut stream);
        return;
    }

    match path {
        "/" | "/index.html" => send_html(&mut stream, 200, INDEX_HTML),
        "/app.js" => send_js(&mut stream, 200, APP_JS),
        "/style.css" => send_css(&mut stream, 200, STYLE_CSS),
        "/api/stats" => handle_stats(&mut stream, volume_path),
        "/api/particles" => handle_particles(&mut stream, volume_path),
        "/api/graph" => handle_graph(&mut stream, volume_path),
        p if p.starts_with("/api/search?") => handle_search(&mut stream, volume_path, p),
        _ => send_404(&mut stream),
    }
}

fn handle_stats(stream: &mut TcpStream, volume_path: &PathBuf) {
    let result = (|| -> Result<String, Box<dyn std::error::Error>> {
        let mut store = PersistentStore::open(volume_path)?;
        let _ = store.load_all();
        let info = store.info();
        let particles = store.all_particles();

        let mut dim_counts: HashMap<String, usize> = HashMap::new();
        let mut bond_counts: HashMap<String, usize> = HashMap::new();

        for p in &particles {
            for (name, _) in &p.dimensions {
                *dim_counts.entry(name.clone()).or_insert(0) += 1;
            }
            for bond in &p.gravity {
                let kind = format!("{:?}", bond.kind);
                *bond_counts.entry(kind).or_insert(0) += 1;
            }
        }

        let mut dimensions: Vec<(String, usize)> = dim_counts.into_iter().collect();
        dimensions.sort_by(|a, b| b.1.cmp(&a.1));
        dimensions.truncate(20);

        let mut bond_kinds: Vec<(String, usize)> = bond_counts.into_iter().collect();
        bond_kinds.sort_by(|a, b| b.1.cmp(&a.1));

        let size_bytes = info.total_blocks as u64 * info.block_size as u64;
        let used_bytes = size_bytes * info.used_percent as u64 / 100;
        let free_bytes = size_bytes - used_bytes;

        let stats = VolumeStats {
            label: info.label.clone(),
            version: format!("{}.{}", info.encoding_version >> 16, info.encoding_version & 0xFFFF),
            size_mb: size_bytes / 1024 / 1024,
            used_bytes,
            free_bytes,
            particle_count: particles.len(),
            singularity_count: store.singularity_count(),
            features: Vec::new(),
            dimensions,
            bond_kinds,
        };

        Ok(serde_json::to_string(&stats)?)
    })();

    match result {
        Ok(json) => send_json(stream, 200, &json),
        Err(e) => {
            eprintln!("Stats error: {}", e);
            send_json(stream, 500, r#"{"error":"failed to load volume"}"#);
        }
    }
}

fn handle_particles(stream: &mut TcpStream, volume_path: &PathBuf) {
    let result = (|| -> Result<String, Box<dyn std::error::Error>> {
        let mut store = PersistentStore::open(volume_path)?;
        let _ = store.load_all();
        let particles = store.all_particles();

        let mut summaries: Vec<ParticleSummary> = particles
            .into_iter()
            .map(|p| ParticleSummary {
                id: p.id.to_hex(),
                name: p.name().map(|s| s.to_string()),
                content_type: p.dimension("content_type").and_then(|w| w.as_str()).map(|s| s.to_string()),
                created_at: p.created_at_ns,
                modified_at: p.modified_at_ns,
                dimension_count: p.dimensions.len(),
                bond_count: p.gravity.len(),
                incoming_count: store.incoming_bonds(&p.id, None).map(|v| v.len()).unwrap_or(0),
            })
            .collect();

        summaries.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        summaries.truncate(100);

        Ok(serde_json::to_string(&summaries)?)
    })();

    match result {
        Ok(json) => send_json(stream, 200, &json),
        Err(e) => {
            eprintln!("Particles error: {}", e);
            send_json(stream, 500, r#"{"error":"failed to load particles"}"#);
        }
    }
}

fn handle_graph(stream: &mut TcpStream, volume_path: &PathBuf) {
    let result = (|| -> Result<String, Box<dyn std::error::Error>> {
        let mut store = PersistentStore::open(volume_path)?;
        let _ = store.load_all();
        let particles = store.all_particles();

        let mut nodes = Vec::new();
        let mut links = Vec::new();

        for p in &particles {
            let name = p.name().unwrap_or(&p.id.to_hex()[..8]).to_string();
            let ct = p.dimension("content_type").and_then(|w| w.as_str()).unwrap_or("");
            let group = if p.dimension("__dir_index").is_some() {
                "directory"
            } else if ct.starts_with("image/") {
                "image"
            } else if ct.starts_with("text/") {
                "text"
            } else {
                "file"
            }
            .to_string();

            nodes.push(GraphNode {
                id: p.id.to_hex(),
                name,
                group,
            });

            for bond in &p.gravity {
                links.push(GraphLink {
                    source: p.id.to_hex(),
                    target: bond.target.to_hex(),
                    kind: format!("{:?}", bond.kind),
                    strength: bond.strength as f64,
                });
            }
        }

        let graph = GraphData { nodes, links };
        Ok(serde_json::to_string(&graph)?)
    })();

    match result {
        Ok(json) => send_json(stream, 200, &json),
        Err(e) => {
            eprintln!("Graph error: {}", e);
            send_json(stream, 500, r#"{"error":"failed to load graph"}"#);
        }
    }
}

fn handle_search(stream: &mut TcpStream, volume_path: &PathBuf, url: &str) {
    let query = url.split('?').nth(1).unwrap_or("");
    let params: HashMap<String, String> = query
        .split('&')
        .filter_map(|p| {
            let mut parts = p.splitn(2, '=');
            let k = parts.next()?;
            let v = parts.next()?;
            Some((k.to_string(), v.to_string()))
        })
        .collect();

    let q = params.get("q").cloned().unwrap_or_default();
    let kind = params.get("kind").cloned().unwrap_or_else(|| "contains".to_string());
    let dim = params.get("dim").cloned().unwrap_or_else(|| "name".to_string());

    let result = (|| -> Result<String, Box<dyn std::error::Error>> {
        let mut store = PersistentStore::open(volume_path)?;
        let _ = store.load_all();

        let results = match kind.as_str() {
            "equals" => store.search(&SearchQuery::DimensionEquals {
                name: dim,
                value: defs_core::particle::Wavelet::from_string(&q),
            })?,
            "related" => {
                let id = defs_core::particle::ParticleId::from_hex(&q)
                    .ok_or("invalid particle id")?;
                store.search(&SearchQuery::RelatedTo {
                    id,
                    kind: None,
                    max_depth: 2,
                })?
            }
            "semantic" => {
                // Placeholder — will integrate when semantic search agent completes
                Vec::new()
            }
            _ => store.search(&SearchQuery::DimensionContains {
                name: dim,
                substring: q.clone(),
            })?,
        };

        let summaries: Vec<ParticleSummary> = results
            .into_iter()
            .map(|p| ParticleSummary {
                id: p.id.to_hex(),
                name: p.name().map(|s| s.to_string()),
                content_type: p.dimension("content_type").and_then(|w| w.as_str()).map(|s| s.to_string()),
                created_at: p.created_at_ns,
                modified_at: p.modified_at_ns,
                dimension_count: p.dimensions.len(),
                bond_count: p.gravity.len(),
                incoming_count: store.incoming_bonds(&p.id, None).map(|v| v.len()).unwrap_or(0),
            })
            .collect();

        Ok(serde_json::to_string(&summaries)?)
    })();

    match result {
        Ok(json) => send_json(stream, 200, &json),
        Err(e) => {
            eprintln!("Search error: {}", e);
            send_json(stream, 500, &format!(r#"{{"error":"{}"}}"#, e));
        }
    }
}

fn send_html(stream: &mut TcpStream, status: u16, body: &str) {
    send(stream, status, "text/html; charset=utf-8", body);
}

fn send_css(stream: &mut TcpStream, status: u16, body: &str) {
    send(stream, status, "text/css; charset=utf-8", body);
}

fn send_js(stream: &mut TcpStream, status: u16, body: &str) {
    send(stream, status, "application/javascript; charset=utf-8", body);
}

fn send_json(stream: &mut TcpStream, status: u16, body: &str) {
    send(stream, status, "application/json; charset=utf-8", body);
}

fn send(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let status_line = match status {
        200 => "HTTP/1.1 200 OK",
        404 => "HTTP/1.1 404 Not Found",
        405 => "HTTP/1.1 405 Method Not Allowed",
        _ => "HTTP/1.1 500 Internal Server Error",
    };
    let response = format!(
        "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status_line,
        content_type,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
}

fn send_404(stream: &mut TcpStream) {
    send_html(stream, 404, "<h1>404 Not Found</h1>");
}

fn send_405(stream: &mut TcpStream) {
    send_html(stream, 405, "<h1>405 Method Not Allowed</h1>");
}
