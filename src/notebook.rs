use crate::ansi::ansi_to_html;
use crate::cache::Blob;
use crate::fs_tree::resolve_relative_asset;
use crate::highlight::{highlight_code, html_escape};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use latex2mathml::{latex_to_mathml, DisplayStyle};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use pulldown_cmark::{html as cm_html, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::Instant;

#[derive(Serialize)]
pub struct RenderedPayload {
    pub doc_id: u64,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub kernel: String,
    pub cell_count: usize,
    pub file_size: u64,
    pub parse_ms: u64,
    pub html: String,
}

pub struct RenderOutput {
    pub payload_json: String,
    pub blobs: Vec<Blob>,
}

const MAX_TEXT_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn url_encode_query(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn value_to_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => {
            let mut total = 0;
            for item in arr {
                if let Some(s) = item.as_str() {
                    total += s.len();
                }
            }
            let mut out = String::with_capacity(total);
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push_str(s);
                }
            }
            out
        }
        _ => String::new(),
    }
}

fn decode_base64_clean(raw: &str) -> Option<Vec<u8>> {
    let cleaned: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    BASE64.decode(cleaned.as_bytes()).ok()
}

fn format_exec_count(v: Option<&Value>) -> String {
    match v {
        Some(Value::Number(n)) => n.to_string(),
        _ => " ".to_string(),
    }
}

fn build_sanitizer() -> ammonia::Builder<'static> {
    let mut builder = ammonia::Builder::default();
    let extra_tags = [
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "caption",
        "colgroup",
        "col",
        "details",
        "summary",
        "figure",
        "figcaption",
        "mark",
        "del",
        "ins",
        "sub",
        "sup",
        "input",
        "section",
        "span",
        "div",
        "pre",
        "code",
        "br",
        "hr",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "blockquote",
        "ul",
        "ol",
        "li",
        "dl",
        "dt",
        "dd",
        "p",
        "a",
        "img",
        "strong",
        "em",
        "b",
        "i",
        "u",
        "s",
        "kbd",
        "var",
        "samp",
    ];
    builder.add_tags(extra_tags);
    builder.add_generic_attributes([
        "class",
        "title",
        "align",
        "valign",
        "colspan",
        "rowspan",
        "width",
        "height",
        "id",
        "scope",
    ]);
    builder.add_tag_attributes("img", ["src", "alt", "loading", "decoding"]);
    builder.add_tag_attributes("a", ["href", "target"]);
    builder.add_tag_attributes("input", ["type", "checked", "disabled"]);
    builder.add_tag_attributes("details", ["open"]);
    builder.add_url_schemes(["http", "https", "mailto", "data"]);
    builder.url_relative(ammonia::UrlRelative::PassThrough);
    builder.clean_content_tags(HashSet::from(["script", "style", "iframe", "object", "embed"]));
    builder
}

fn render_latex_mathml(latex: &str, display: bool) -> String {
    let trimmed = latex.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let style = if display {
        DisplayStyle::Block
    } else {
        DisplayStyle::Inline
    };
    match latex_to_mathml(trimmed, style) {
        Ok(mathml) => {
            if display {
                format!("<div class=\"nb-math-block\">{mathml}</div>")
            } else {
                format!("<span class=\"nb-math-inline\">{mathml}</span>")
            }
        }
        Err(_) => {
            if display {
                format!("<pre class=\"nb-math-fallback\">$$</pre>", )
                    .replace("$$", &html_escape(trimmed))
            } else {
                format!("<code class=\"nb-math-fallback\">{}</code>", html_escape(trimmed))
            }
        }
    }
}

struct AttachmentLookup<'a> {
    attachments: Option<&'a serde_json::Map<String, Value>>,
    doc_id: u64,
    blobs: &'a mut Vec<Blob>,
    root: &'a Path,
    doc_rel_path: &'a str,
}

impl<'a> AttachmentLookup<'a> {
    fn resolve_url(&mut self, url: &str) -> String {
        if let Some(att_name) = url.strip_prefix("attachment:") {
            if let Some(att_map) = self.attachments {
                if let Some(Value::Object(mime_map)) = att_map.get(att_name) {
                    let preferred = [
                        "image/png",
                        "image/jpeg",
                        "image/gif",
                        "image/webp",
                        "image/svg+xml",
                    ];
                    for mime in preferred {
                        if let Some(val) = mime_map.get(mime) {
                            let raw = value_to_string(Some(val));
                            let bytes = if mime == "image/svg+xml" {
                                Some(raw.into_bytes())
                            } else {
                                decode_base64_clean(&raw)
                            };
                            if let Some(data) = bytes {
                                let idx = self.blobs.len();
                                self.blobs.push(Blob {
                                    mime: mime.to_string(),
                                    data,
                                });
                                return format!("/api/blob/{}/{}", self.doc_id, idx);
                            }
                        }
                    }
                }
            }
            return String::new();
        }

        if url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("data:")
            || url.starts_with('#')
            || url.starts_with("/api/")
        {
            return url.to_string();
        }

        if let Some(rel) = resolve_relative_asset(self.root, self.doc_rel_path, url) {
            return format!("/api/raw?path={}", url_encode_query(&rel));
        }

        url.to_string()
    }
}

fn render_markdown_with_ctx(
    source: &str,
    mut ctx: Option<&mut AttachmentLookup<'_>>,
) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;

    let parser = Parser::new_ext(source, options);
    let mut placeholders: Vec<(String, String)> = Vec::new();
    let mut events: Vec<Event<'_>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buf = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_buf.clear();
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                let highlighted = highlight_code(&code_buf, &code_lang);
                let lang_badge = if code_lang.trim().is_empty() {
                    String::new()
                } else {
                    format!(
                        "<span class=\"nb-md-lang\">{}</span>",
                        html_escape(code_lang.trim())
                    )
                };
                let block_html = format!(
                    "<div class=\"nb-md-code-wrap\">{lang_badge}<button class=\"nb-copy-btn\" type=\"button\" title=\"Copy code\">Copy</button><pre class=\"nb-md-code\"><code>{highlighted}</code></pre></div>"
                );
                let token = format!("EOJUPYTERPHCODE{}", placeholders.len());
                placeholders.push((token.clone(), block_html));
                events.push(Event::Html(token.into()));
            }
            Event::Text(text) if in_code_block => {
                code_buf.push_str(&text);
            }
            Event::InlineMath(math) => {
                let math_html = render_latex_mathml(&math, false);
                let token = format!("EOJUPYTERPHMATH{}", placeholders.len());
                placeholders.push((token.clone(), math_html));
                events.push(Event::Html(token.into()));
            }
            Event::DisplayMath(math) => {
                let math_html = render_latex_mathml(&math, true);
                let token = format!("EOJUPYTERPHMATH{}", placeholders.len());
                placeholders.push((token.clone(), math_html));
                events.push(Event::Html(token.into()));
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let new_url = if let Some(ref mut lookup) = ctx {
                    lookup.resolve_url(&dest_url).into()
                } else {
                    dest_url
                };
                events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url: new_url,
                    title,
                    id,
                }));
            }
            other => events.push(other),
        }
    }

    let mut raw_html = String::with_capacity(source.len() * 2);
    cm_html::push_html(&mut raw_html, events.into_iter());

    let sanitizer = build_sanitizer();
    let mut clean = sanitizer.clean(&raw_html).to_string();

    for (token, replacement) in placeholders {
        clean = clean.replace(&token, &replacement);
    }
    clean
}

fn render_rich_data(
    data: &serde_json::Map<String, Value>,
    doc_id: u64,
    blobs: &mut Vec<Blob>,
    sanitizer: &ammonia::Builder<'_>,
) -> String {
    // 1. Raster images
    for mime in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
        if let Some(val) = data.get(mime) {
            let raw = value_to_string(Some(val));
            if let Some(bytes) = decode_base64_clean(&raw) {
                let idx = blobs.len();
                blobs.push(Blob {
                    mime: mime.to_string(),
                    data: bytes,
                });
                return format!(
                    "<div class=\"nb-img-wrap\"><img class=\"nb-img\" src=\"/api/blob/{doc_id}/{idx}\" loading=\"lazy\" decoding=\"async\" alt=\"Cell output image\"></div>"
                );
            }
        }
    }

    // 2. SVG rendered sandboxed via <img>
    if let Some(val) = data.get("image/svg+xml") {
        let svg_str = value_to_string(Some(val));
        if !svg_str.trim().is_empty() {
            let idx = blobs.len();
            blobs.push(Blob {
                mime: "image/svg+xml".to_string(),
                data: svg_str.into_bytes(),
            });
            return format!(
                "<div class=\"nb-img-wrap\"><img class=\"nb-img nb-svg\" src=\"/api/blob/{doc_id}/{idx}\" loading=\"lazy\" decoding=\"async\" alt=\"SVG output\"></div>"
            );
        }
    }

    // 3. HTML output (e.g., pandas DataFrames, IPython widgets fallback HTML)
    if let Some(val) = data.get("text/html") {
        let html_str = value_to_string(Some(val));
        if !html_str.trim().is_empty() {
            let clean = sanitizer.clean(&html_str).to_string();
            return format!("<div class=\"nb-html-output\">{clean}</div>");
        }
    }

    // 4. LaTeX output
    if let Some(val) = data.get("text/latex") {
        let raw = value_to_string(Some(val));
        let stripped = raw
            .trim()
            .trim_start_matches("$$")
            .trim_end_matches("$$")
            .trim_start_matches('$')
            .trim_end_matches('$')
            .trim_start_matches("\\[")
            .trim_end_matches("\\]");
        if !stripped.trim().is_empty() {
            return render_latex_mathml(stripped, true);
        }
    }

    // 5. Markdown output
    if let Some(val) = data.get("text/markdown") {
        let md = value_to_string(Some(val));
        if !md.trim().is_empty() {
            let rendered = render_markdown_with_ctx(&md, None);
            return format!("<div class=\"nb-md-body\">{rendered}</div>");
        }
    }

    // 6. JSON output
    if let Some(val) = data.get("application/json") {
        let pretty = serde_json::to_string_pretty(val).unwrap_or_else(|_| val.to_string());
        let hl = highlight_code(&pretty, "json");
        return format!("<pre class=\"nb-code nb-json-output\"><code>{hl}</code></pre>");
    }

    // 7. Plain text fallback
    if let Some(val) = data.get("text/plain") {
        let txt = value_to_string(Some(val));
        let escaped = ansi_to_html(&txt);
        return format!("<pre class=\"nb-text-output\">{escaped}</pre>");
    }

    // 8. Known interactive / unsupported MIME notice
    if data.keys().any(|k| k.contains("vnd.jupyter.widget")) {
        return "<div class=\"nb-muted-note\">[Interactive Jupyter widget output not available in read-only mode]</div>".to_string();
    }

    String::new()
}

fn render_cell_outputs(
    outputs: &[Value],
    doc_id: u64,
    blobs: &mut Vec<Blob>,
    sanitizer: &ammonia::Builder<'_>,
) -> String {
    let mut html = String::new();
    let mut i = 0;

    while i < outputs.len() {
        let out_obj = &outputs[i];
        let out_type = out_obj
            .get("output_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match out_type {
            "stream" => {
                let stream_name = out_obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("stdout");
                let mut combined = value_to_string(out_obj.get("text"));
                let mut j = i + 1;
                while j < outputs.len() {
                    let next = &outputs[j];
                    let next_type = next.get("output_type").and_then(|v| v.as_str()).unwrap_or("");
                    let next_name = next.get("name").and_then(|v| v.as_str()).unwrap_or("stdout");
                    if next_type == "stream" && next_name == stream_name {
                        combined.push_str(&value_to_string(next.get("text")));
                        j += 1;
                    } else {
                        break;
                    }
                }
                i = j;
                let cls = if stream_name == "stderr" {
                    "nb-stream nb-stderr"
                } else {
                    "nb-stream nb-stdout"
                };
                let body = ansi_to_html(&combined);
                let _ = write!(
                    html,
                    "<div class=\"nb-output-row\"><div class=\"nb-gutter\"></div><div class=\"nb-output-main\"><pre class=\"{cls}\">{body}</pre></div></div>"
                );
            }
            "execute_result" | "display_data" => {
                let exec_label = if out_type == "execute_result" {
                    let n = format_exec_count(
                        out_obj
                            .get("execution_count")
                            .or_else(|| out_obj.get("prompt_number")),
                    );
                    format!("Out[{n}]:")
                } else {
                    String::new()
                };
                let rendered = out_obj
                    .get("data")
                    .and_then(|v| v.as_object())
                    .map(|map| render_rich_data(map, doc_id, blobs, sanitizer))
                    .unwrap_or_default();
                if !rendered.is_empty() {
                    let _ = write!(
                        html,
                        "<div class=\"nb-output-row\"><div class=\"nb-gutter nb-gutter-out\">{exec_label}</div><div class=\"nb-output-main\">{rendered}</div></div>"
                    );
                }
                i += 1;
            }
            "error" => {
                let ename = out_obj
                    .get("ename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Error");
                let evalue = out_obj
                    .get("evalue")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let traceback = match out_obj.get("traceback") {
                    Some(Value::Array(lines)) => lines
                        .iter()
                        .filter_map(|l| l.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    Some(Value::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let head = format!("{}: {}", html_escape(ename), html_escape(evalue));
                let tb_html = ansi_to_html(&traceback);
                let _ = write!(
                    html,
                    "<div class=\"nb-output-row\"><div class=\"nb-gutter\"></div><div class=\"nb-output-main\"><div class=\"nb-error\"><div class=\"nb-error-head\">{head}</div><pre class=\"nb-traceback\">{tb_html}</pre></div></div></div>"
                );
                i += 1;
            }
            _ => {
                // Handle nbformat v3 display_data / pyout fallback if present
                let rendered = if let Some(map) = out_obj.as_object() {
                    let mut synth = serde_json::Map::new();
                    for (k, v) in map {
                        match k.as_str() {
                            "png" => {
                                synth.insert("image/png".into(), v.clone());
                            }
                            "jpeg" => {
                                synth.insert("image/jpeg".into(), v.clone());
                            }
                            "svg" => {
                                synth.insert("image/svg+xml".into(), v.clone());
                            }
                            "html" => {
                                synth.insert("text/html".into(), v.clone());
                            }
                            "latex" => {
                                synth.insert("text/latex".into(), v.clone());
                            }
                            "text" => {
                                synth.insert("text/plain".into(), v.clone());
                            }
                            _ => {}
                        }
                    }
                    if synth.is_empty() {
                        String::new()
                    } else {
                        render_rich_data(&synth, doc_id, blobs, sanitizer)
                    }
                } else {
                    String::new()
                };
                if !rendered.is_empty() {
                    let _ = write!(
                        html,
                        "<div class=\"nb-output-row\"><div class=\"nb-gutter\"></div><div class=\"nb-output-main\">{rendered}</div></div>"
                    );
                }
                i += 1;
            }
        }
    }

    html
}

fn render_notebook_bytes(
    bytes: &[u8],
    root: &Path,
    rel_path: &str,
    file_name: &str,
    file_size: u64,
    doc_id: u64,
) -> Result<RenderOutput, String> {
    let start = Instant::now();
    let nb: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid notebook JSON: {e}"))?;

    let meta = nb.get("metadata");
    let kernel_display = meta
        .and_then(|m| m.get("kernelspec"))
        .and_then(|k| k.get("display_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let kernel_lang = meta
        .and_then(|m| m.get("kernelspec"))
        .and_then(|k| k.get("language"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            meta.and_then(|m| m.get("language_info"))
                .and_then(|l| l.get("name"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("python");

    let kernel_label = if !kernel_display.is_empty() {
        kernel_display.to_string()
    } else {
        kernel_lang.to_string()
    };

    let cells_slice: Vec<Value> = if let Some(arr) = nb.get("cells").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(ws) = nb
        .get("worksheets")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|w| w.get("cells"))
        .and_then(|v| v.as_array())
    {
        ws.clone()
    } else {
        Vec::new()
    };

    let cell_count = cells_slice.len();
    let mut blobs = Vec::new();
    let sanitizer = build_sanitizer();
    let mut html = String::with_capacity(bytes.len().min(512 * 1024));

    if cells_slice.is_empty() {
        html.push_str("<div class=\"nb-empty\">This notebook has 0 cells.</div>");
    }

    for (idx, cell) in cells_slice.iter().enumerate() {
        let cell_type = cell
            .get("cell_type")
            .and_then(|v| v.as_str())
            .unwrap_or("code");
        let source = value_to_string(
            cell.get("source")
                .or_else(|| cell.get("input")),
        );

        match cell_type {
            "markdown" | "heading" => {
                let md_src = if cell_type == "heading" {
                    let level = cell.get("level").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 6);
                    format!("{} {}", "#".repeat(level as usize), source)
                } else {
                    source
                };
                let attachments = cell.get("attachments").and_then(|v| v.as_object());
                let mut lookup = AttachmentLookup {
                    attachments,
                    doc_id,
                    blobs: &mut blobs,
                    root,
                    doc_rel_path: rel_path,
                };
                let md_html = render_markdown_with_ctx(&md_src, Some(&mut lookup));
                let _ = write!(
                    html,
                    "<section class=\"nb-cell nb-cell-markdown\" data-cell-index=\"{idx}\"><div class=\"nb-gutter\"></div><div class=\"nb-md-body\">{md_html}</div></section>"
                );
            }
            "raw" => {
                let escaped = html_escape(&source);
                let _ = write!(
                    html,
                    "<section class=\"nb-cell nb-cell-raw\" data-cell-index=\"{idx}\"><div class=\"nb-gutter nb-gutter-raw\">Raw</div><pre class=\"nb-raw\">{escaped}</pre></section>"
                );
            }
            _ => {
                let exec_n = format_exec_count(
                    cell.get("execution_count")
                        .or_else(|| cell.get("prompt_number")),
                );
                let highlighted = highlight_code(&source, kernel_lang);
                let outputs_html = cell
                    .get("outputs")
                    .and_then(|v| v.as_array())
                    .map(|outs| render_cell_outputs(outs, doc_id, &mut blobs, &sanitizer))
                    .unwrap_or_default();

                let _ = write!(
                    html,
                    "<section class=\"nb-cell nb-cell-code\" data-cell-index=\"{idx}\">\
                        <div class=\"nb-input-row\">\
                            <div class=\"nb-gutter nb-gutter-in\">In [{exec_n}]:</div>\
                            <div class=\"nb-code-wrap\">\
                                <button class=\"nb-copy-btn\" type=\"button\" title=\"Copy code\">Copy</button>\
                                <pre class=\"nb-code\"><code>{highlighted}</code></pre>\
                            </div>\
                        </div>\
                        {outputs_html}\
                    </section>"
                );
            }
        }
    }

    let parse_ms = start.elapsed().as_millis() as u64;
    let payload = RenderedPayload {
        doc_id,
        path: rel_path.to_string(),
        name: file_name.to_string(),
        kind: "notebook".to_string(),
        kernel: kernel_label,
        cell_count,
        file_size,
        parse_ms,
        html,
    };
    let payload_json =
        serde_json::to_string(&payload).map_err(|e| format!("JSON serialize error: {e}"))?;

    Ok(RenderOutput {
        payload_json,
        blobs,
    })
}

pub fn render_file(
    root: &Path,
    abs_path: &Path,
    rel_path: &str,
    doc_id: u64,
    file_size: u64,
) -> Result<RenderOutput, String> {
    let file_name = abs_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(rel_path)
        .to_string();
    let ext = abs_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "ipynb" {
        let bytes = fs::read(abs_path).map_err(|e| format!("Failed to read file: {e}"))?;
        return render_notebook_bytes(&bytes, root, rel_path, &file_name, file_size, doc_id);
    }

    let start = Instant::now();

    if matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
    ) {
        let encoded = url_encode_query(rel_path);
        let html = format!(
            "<div class=\"file-preview-image\"><img src=\"/api/raw?path={encoded}\" alt=\"{}\"></div>",
            html_escape(&file_name)
        );
        let payload = RenderedPayload {
            doc_id,
            path: rel_path.to_string(),
            name: file_name,
            kind: "image".to_string(),
            kernel: ext.to_uppercase(),
            cell_count: 0,
            file_size,
            parse_ms: start.elapsed().as_millis() as u64,
            html,
        };
        return Ok(RenderOutput {
            payload_json: serde_json::to_string(&payload).unwrap(),
            blobs: Vec::new(),
        });
    }

    if file_size > MAX_TEXT_FILE_BYTES {
        let html = format!(
            "<div class=\"nb-empty\">File is too large for preview ({:.1} MB).</div>",
            file_size as f64 / (1024.0 * 1024.0)
        );
        let payload = RenderedPayload {
            doc_id,
            path: rel_path.to_string(),
            name: file_name,
            kind: "binary".to_string(),
            kernel: "Binary / Large".to_string(),
            cell_count: 0,
            file_size,
            parse_ms: start.elapsed().as_millis() as u64,
            html,
        };
        return Ok(RenderOutput {
            payload_json: serde_json::to_string(&payload).unwrap(),
            blobs: Vec::new(),
        });
    }

    let bytes = fs::read(abs_path).map_err(|e| format!("Failed to read file: {e}"))?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        let html = "<div class=\"nb-empty\">Binary file preview is not supported.</div>".to_string();
        let payload = RenderedPayload {
            doc_id,
            path: rel_path.to_string(),
            name: file_name,
            kind: "binary".to_string(),
            kernel: "Binary".to_string(),
            cell_count: 0,
            file_size,
            parse_ms: start.elapsed().as_millis() as u64,
            html,
        };
        return Ok(RenderOutput {
            payload_json: serde_json::to_string(&payload).unwrap(),
            blobs: Vec::new(),
        });
    };

    if matches!(ext.as_str(), "md" | "markdown") {
        let mut blobs = Vec::new();
        let mut lookup = AttachmentLookup {
            attachments: None,
            doc_id,
            blobs: &mut blobs,
            root,
            doc_rel_path: rel_path,
        };
        let md_html = render_markdown_with_ctx(text, Some(&mut lookup));
        let html = format!("<article class=\"file-preview-md nb-md-body\">{md_html}</article>");
        let payload = RenderedPayload {
            doc_id,
            path: rel_path.to_string(),
            name: file_name,
            kind: "markdown".to_string(),
            kernel: "Markdown".to_string(),
            cell_count: 0,
            file_size,
            parse_ms: start.elapsed().as_millis() as u64,
            html,
        };
        return Ok(RenderOutput {
            payload_json: serde_json::to_string(&payload).unwrap(),
            blobs,
        });
    }

    let highlighted = highlight_code(text, &ext);
    let line_count = text.lines().count().max(1);
    let mut gutter = String::with_capacity(line_count * 6);
    for n in 1..=line_count {
        let _ = writeln!(gutter, "{n}");
    }
    let html = format!(
        "<div class=\"file-preview-code\"><pre class=\"file-line-nums\">{gutter}</pre><pre class=\"nb-code file-code-body\"><code>{highlighted}</code></pre></div>"
    );
    let payload = RenderedPayload {
        doc_id,
        path: rel_path.to_string(),
        name: file_name,
        kind: "text".to_string(),
        kernel: if ext.is_empty() {
            "Text".to_string()
        } else {
            ext.to_uppercase()
        },
        cell_count: line_count,
        file_size,
        parse_ms: start.elapsed().as_millis() as u64,
        html,
    };
    Ok(RenderOutput {
        payload_json: serde_json::to_string(&payload).unwrap(),
        blobs: Vec::new(),
    })
}

/// Helper used by tests or internal callers to inspect rendered notebook JSON directly.
#[allow(dead_code)]
pub fn render_notebook_for_test(json: &str, root: &Path) -> Result<(RenderedPayload, String, Vec<Blob>), String> {
    let out = render_notebook_bytes(json.as_bytes(), root, "test.ipynb", "test.ipynb", json.len() as u64, 1)?;
    let parsed: Value = serde_json::from_str(&out.payload_json).unwrap();
    let html = parsed["html"].as_str().unwrap_or("").to_string();
    let payload = RenderedPayload {
        doc_id: 1,
        path: "test.ipynb".into(),
        name: "test.ipynb".into(),
        kind: "notebook".into(),
        kernel: parsed["kernel"].as_str().unwrap_or("").into(),
        cell_count: parsed["cell_count"].as_u64().unwrap_or(0) as usize,
        file_size: json.len() as u64,
        parse_ms: 0,
        html: html.clone(),
    };
    Ok((payload, html, out.blobs))
}

/// Unused HashMap suppression not needed; keep import clean
#[allow(dead_code)]
fn _dummy(_: HashMap<String, String>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_v4_notebook_with_math_xss_sanitization_and_blobs() {
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let nb = serde_json::json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {
                "kernelspec": { "display_name": "Python 3", "language": "python" }
            },
            "cells": [
                {
                    "cell_type": "markdown",
                    "metadata": {},
                    "attachments": {
                        "tiny.png": { "image/png": png_b64 }
                    },
                    "source": [
                        "# Title\n",
                        "Math $x^2 + y^2 = z^2$ and ![att](attachment:tiny.png)\n",
                        "<script>alert('xss')</script><img src=x onerror=\"alert(1)\">"
                    ]
                },
                {
                    "cell_type": "code",
                    "execution_count": 7,
                    "metadata": {},
                    "source": ["import math\nprint(math.pi)"],
                    "outputs": [
                        { "output_type": "stream", "name": "stdout", "text": ["3.14159\n"] },
                        {
                            "output_type": "execute_result",
                            "execution_count": 7,
                            "data": {
                                "image/png": png_b64,
                                "text/plain": ["<Figure>"]
                            }
                        },
                        {
                            "output_type": "display_data",
                            "data": {
                                "image/svg+xml": ["<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(2)</script><circle r=\"5\"/></svg>"]
                            }
                        },
                        {
                            "output_type": "display_data",
                            "data": {
                                "text/html": ["<table class=\"dataframe\"><tr><th>A</th></tr><tr><td>1</td></tr></table><script>evil()</script>"]
                            }
                        }
                    ]
                }
            ]
        });

        let (_, html, blobs) =
            render_notebook_for_test(&nb.to_string(), Path::new("/tmp")).unwrap();
        assert!(html.contains("In [7]:"));
        assert!(html.contains("Out[7]:"));
        assert!(html.contains("<math"));
        assert!(html.contains("class=\"dataframe\""));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("onerror"));
        assert_eq!(blobs.len(), 3);
        assert_eq!(blobs[0].mime, "image/png");
        assert_eq!(blobs[1].mime, "image/png");
        assert_eq!(blobs[2].mime, "image/svg+xml");
        assert!(html.contains("/api/blob/1/0"));
        assert!(html.contains("/api/blob/1/1"));
        assert!(html.contains("/api/blob/1/2"));
    }

    #[test]
    fn renders_v3_worksheets_fallback() {
        let nb = r#"{
            "nbformat": 3,
            "worksheets": [{
                "cells": [
                    { "cell_type": "heading", "level": 2, "source": ["Legacy"] },
                    { "cell_type": "code", "prompt_number": 3, "input": ["x = 1"], "outputs": [] }
                ]
            }]
        }"#;
        let (payload, html, _) = render_notebook_for_test(nb, Path::new("/tmp")).unwrap();
        assert_eq!(payload.cell_count, 2);
        assert!(html.contains("<h2>Legacy</h2>"));
        assert!(html.contains("In [3]:"));
    }
}
