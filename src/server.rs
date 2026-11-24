use crate::cache::DocCache;
use crate::fs_tree::{list_dir, resolve_safe_path, search_files};
use crate::highlight::syntax_css;
use crate::notebook::render_file;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use tiny_http::{Header, Method, Request, Response, StatusCode};

const INDEX_HTML: &str = include_str!("../web/index.html");
const STYLE_CSS: &str = include_str!("../web/style.css");
const APP_JS: &str = include_str!("../web/app.js");

pub struct AppState {
    pub root: RwLock<PathBuf>,
    pub initial_file: RwLock<Option<String>>,
    pub cache: DocCache,
}

impl AppState {
    pub fn new(root: PathBuf, initial_file: Option<String>) -> Self {
        Self {
            root: RwLock::new(root),
            initial_file: RwLock::new(initial_file),
            cache: DocCache::new(40, 256 * 1024 * 1024),
        }
    }
}

#[derive(Serialize)]
struct InfoResponse {
    root: String,
    root_name: String,
    initial_file: Option<String>,
    version: &'static str,
}

#[derive(Deserialize)]
struct SetRootReq {
    path: String,
}

#[derive(Deserialize)]
struct PrefetchReq {
    paths: Vec<String>,
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid HTTP header")
}

fn respond_text(req: Request, status: u16, content_type: &str, body: &str) {
    let resp = Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", content_type));
    let _ = req.respond(resp);
}

fn respond_bytes(
    req: Request,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    cache_control: Option<&str>,
) {
    let mut resp = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", content_type));
    if let Some(cc) = cache_control {
        resp = resp.with_header(header("Cache-Control", cc));
    }
    let _ = req.respond(resp);
}

fn respond_json_error(req: Request, status: u16, message: &str) {
    let body = serde_json::json!({ "error": message }).to_string();
    respond_text(req, status, "application/json; charset=utf-8", &body);
}

fn parse_query(url: &str) -> (&str, Vec<(String, String)>) {
    let Some((path, query)) = url.split_once('?') else {
        return (url, Vec::new());
    };
    let mut params = Vec::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let dk = percent_decode_str(&k.replace('+', " "))
            .decode_utf8_lossy()
            .to_string();
        let dv = percent_decode_str(&v.replace('+', " "))
            .decode_utf8_lossy()
            .to_string();
        params.push((dk, dv));
    }
    (path, params)
}

fn query_param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn query_bool(params: &[(String, String)], key: &str, default: bool) -> bool {
    match query_param(params, key) {
        Some("1") | Some("true") | Some("yes") => true,
        Some("0") | Some("false") | Some("no") => false,
        _ => default,
    }
}

fn read_json_body<T: for<'de> Deserialize<'de>>(req: &mut Request) -> Result<T, String> {
    let mut buf = String::new();
    req.as_reader()
        .take(1024 * 1024)
        .read_to_string(&mut buf)
        .map_err(|e| format!("Failed reading request body: {e}"))?;
    serde_json::from_str(&buf).map_err(|e| format!("Invalid JSON body: {e}"))
}

fn render_or_cached(
    state: &AppState,
    root: &Path,
    rel_path: &str,
    nocache: bool,
) -> Result<Arc<String>, (u16, String)> {
    let abs = resolve_safe_path(root, rel_path).map_err(|e| (404, e))?;
    if !abs.is_file() {
        return Err((400, "Target path is not a file".to_string()));
    }
    let meta = fs::metadata(&abs).map_err(|e| (500, format!("Stat failed: {e}")))?;
    let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
    let len = meta.len();

    if !nocache {
        if let Some(cached) = state.cache.get(&abs, modified, len) {
            return Ok(cached);
        }
    }

    let doc_id = state.cache.alloc_doc_id();
    let rendered = render_file(root, &abs, rel_path, doc_id, len).map_err(|e| (500, e))?;
    let payload_arc = state.cache.insert(
        abs,
        doc_id,
        modified,
        len,
        rendered.payload_json,
        rendered.blobs,
    );
    Ok(payload_arc)
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "json" => "application/json; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "md" | "txt" | "py" | "rs" | "csv" | "tsv" | "yaml" | "yml" | "toml" => {
            "text/plain; charset=utf-8"
        }
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn handle_request(mut req: Request, state: &Arc<AppState>) {
    let url = req.url().to_string();
    let method = req.method().clone();
    let (path, params) = parse_query(&url);

    match (method, path) {
        (Method::Get, "/") => {
            respond_text(req, 200, "text/html; charset=utf-8", INDEX_HTML);
        }
        (Method::Get, "/style.css") => {
            respond_text(req, 200, "text/css; charset=utf-8", STYLE_CSS);
        }
        (Method::Get, "/syntax.css") => {
            respond_text(req, 200, "text/css; charset=utf-8", syntax_css());
        }
        (Method::Get, "/app.js") => {
            respond_text(req, 200, "application/javascript; charset=utf-8", APP_JS);
        }
        (Method::Get, "/api/info") => {
            let root = state.root.read().unwrap().clone();
            let initial_file = state.initial_file.read().unwrap().clone();
            let root_name = root
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| root.display().to_string());
            let info = InfoResponse {
                root: root.display().to_string(),
                root_name,
                initial_file,
                version: env!("CARGO_PKG_VERSION"),
            };
            respond_text(
                req,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string(&info).unwrap(),
            );
        }
        (Method::Post, "/api/root") => {
            let body: SetRootReq = match read_json_body(&mut req) {
                Ok(b) => b,
                Err(e) => {
                    respond_json_error(req, 400, &e);
                    return;
                }
            };
            let candidate = PathBuf::from(body.path.trim());
            let Ok(canonical) = fs::canonicalize(&candidate) else {
                respond_json_error(req, 400, "Directory does not exist");
                return;
            };
            if !canonical.is_dir() {
                respond_json_error(req, 400, "Path is not a directory");
                return;
            }
            {
                let mut guard = state.root.write().unwrap();
                *guard = canonical.clone();
            }
            {
                let mut init_guard = state.initial_file.write().unwrap();
                *init_guard = None;
            }
            state.cache.clear();
            let root_name = canonical
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| canonical.display().to_string());
            let info = InfoResponse {
                root: canonical.display().to_string(),
                root_name,
                initial_file: None,
                version: env!("CARGO_PKG_VERSION"),
            };
            respond_text(
                req,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string(&info).unwrap(),
            );
        }
        (Method::Get, "/api/list") => {
            let rel = query_param(&params, "path").unwrap_or("");
            let notebooks_only = query_bool(&params, "notebooks_only", true);
            let show_hidden = query_bool(&params, "show_hidden", false);
            let root = state.root.read().unwrap().clone();
            match list_dir(&root, rel, notebooks_only, show_hidden) {
                Ok(entries) => respond_text(
                    req,
                    200,
                    "application/json; charset=utf-8",
                    &serde_json::to_string(&entries).unwrap(),
                ),
                Err(e) => respond_json_error(req, 400, &e),
            }
        }
        (Method::Get, "/api/search") => {
            let q = query_param(&params, "q").unwrap_or("");
            let notebooks_only = query_bool(&params, "notebooks_only", true);
            let show_hidden = query_bool(&params, "show_hidden", false);
            let root = state.root.read().unwrap().clone();
            let entries = search_files(&root, q, notebooks_only, show_hidden, 200);
            respond_text(
                req,
                200,
                "application/json; charset=utf-8",
                &serde_json::to_string(&entries).unwrap(),
            );
        }
        (Method::Get, "/api/render") => {
            let Some(rel) = query_param(&params, "path") else {
                respond_json_error(req, 400, "Missing path query parameter");
                return;
            };
            let nocache = query_bool(&params, "nocache", false);
            let root = state.root.read().unwrap().clone();
            match render_or_cached(state, &root, rel, nocache) {
                Ok(payload_arc) => {
                    respond_text(req, 200, "application/json; charset=utf-8", &payload_arc);
                }
                Err((status, msg)) => respond_json_error(req, status, &msg),
            }
        }
        (Method::Post, "/api/prefetch") => {
            if let Ok(body) = read_json_body::<PrefetchReq>(&mut req) {
                let state_bg = Arc::clone(state);
                let paths = body.paths.into_iter().take(4).collect::<Vec<_>>();
                thread::spawn(move || {
                    let root = state_bg.root.read().unwrap().clone();
                    for p in paths {
                        let _ = render_or_cached(&state_bg, &root, &p, false);
                    }
                });
            }
            respond_bytes(req, 204, "text/plain", Vec::new(), None);
        }
        (Method::Get, p) if p.starts_with("/api/blob/") => {
            let rest = &p["/api/blob/".len()..];
            let mut parts = rest.split('/');
            let doc_id = parts.next().and_then(|s| s.parse::<u64>().ok());
            let blob_idx = parts.next().and_then(|s| s.parse::<usize>().ok());
            match (doc_id, blob_idx) {
                (Some(did), Some(bidx)) => {
                    if let Some((mime, data)) = state.cache.get_blob(did, bidx) {
                        respond_bytes(req, 200, &mime, data, Some("public, max-age=3600"));
                    } else {
                        respond_json_error(req, 404, "Blob not found");
                    }
                }
                _ => respond_json_error(req, 400, "Invalid blob URL"),
            }
        }
        (Method::Get, "/api/raw") => {
            let Some(rel) = query_param(&params, "path") else {
                respond_json_error(req, 400, "Missing path query parameter");
                return;
            };
            let root = state.root.read().unwrap().clone();
            let abs = match resolve_safe_path(&root, rel) {
                Ok(p) => p,
                Err(e) => {
                    respond_json_error(req, 404, &e);
                    return;
                }
            };
            if !abs.is_file() {
                respond_json_error(req, 400, "Not a file");
                return;
            }
            match fs::read(&abs) {
                Ok(data) => {
                    let mime = mime_for_path(&abs);
                    respond_bytes(req, 200, mime, data, Some("no-cache"));
                }
                Err(e) => respond_json_error(req, 500, &format!("Read error: {e}")),
            }
        }
        _ => {
            respond_json_error(req, 404, "Not found");
        }
    }
}

/// Starts the local HTTP server on `127.0.0.1:<port>` (use `0` for OS-assigned ephemeral port).
pub fn start_server(state: Arc<AppState>, port: u16) -> Result<SocketAddr, String> {
    let bind_addr = format!("127.0.0.1:{port}");
    let server = Arc::new(
        tiny_http::Server::http(&bind_addr)
            .map_err(|e| format!("Failed to bind HTTP server on {bind_addr}: {e}"))?,
    );
    let local_addr = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "Server did not bind to an IP socket".to_string())?;

    let worker_count = 6;
    for idx in 0..worker_count {
        let srv = Arc::clone(&server);
        let st = Arc::clone(&state);
        thread::Builder::new()
            .name(format!("http-worker-{idx}"))
            .spawn(move || {
                for req in srv.incoming_requests() {
                    handle_request(req, &st);
                }
            })
            .map_err(|e| format!("Failed to spawn server worker: {e}"))?;
    }

    Ok(local_addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpStream;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).unwrap();
        let text = String::from_utf8_lossy(&resp).to_string();
        let status = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        let body = text.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string();
        (status, body)
    }

    #[test]
    fn end_to_end_server_routes_work() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("eojupyter_srv_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let nb = serde_json::json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": { "kernelspec": { "display_name": "Python 3", "language": "python" } },
            "cells": [
                { "cell_type": "markdown", "source": ["# Hello"] },
                {
                    "cell_type": "code",
                    "execution_count": 1,
                    "source": ["print(1)"],
                    "outputs": [{ "output_type": "display_data", "data": { "image/png": png_b64 } }]
                }
            ]
        });
        fs::write(dir.join("demo.ipynb"), nb.to_string()).unwrap();

        let state = Arc::new(AppState::new(dir.clone(), Some("demo.ipynb".into())));
        let addr = start_server(state, 0).unwrap();

        let (s_home, home) = http_get(addr, "/");
        assert_eq!(s_home, 200);
        assert!(home.contains("eojupyter"));

        let (s_info, info) = http_get(addr, "/api/info");
        assert_eq!(s_info, 200);
        assert!(info.contains("demo.ipynb"));

        let (s_list, list) = http_get(addr, "/api/list?path=&notebooks_only=1");
        assert_eq!(s_list, 200);
        assert!(list.contains("demo.ipynb"));

        let (s_render, rendered) = http_get(addr, "/api/render?path=demo.ipynb");
        assert_eq!(s_render, 200);
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let doc_id = parsed["doc_id"].as_u64().unwrap();
        assert!(parsed["html"].as_str().unwrap().contains("<h1>Hello</h1>"));

        let (s_blob, _) = http_get(addr, &format!("/api/blob/{doc_id}/0"));
        assert_eq!(s_blob, 200);

        let (s_search, searched) = http_get(addr, "/api/search?q=demo");
        assert_eq!(s_search, 200);
        assert!(searched.contains("demo.ipynb"));

        let _ = fs::remove_dir_all(&dir);
    }
}
