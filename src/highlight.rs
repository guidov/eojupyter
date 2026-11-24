use std::sync::OnceLock;
use syntect::highlighting::ThemeSet;
use syntect::html::{css_for_theme_with_class_style, ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

struct Highlighter {
    syntax_set: SyntaxSet,
    css: String,
}

static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();
const CLASS_STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "syn-" };
const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;

fn scope_dark_syntax_css(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 1024);
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('.') && trimmed.ends_with('{') {
            let selectors = trimmed.trim_end_matches('{').trim();
            let scoped = selectors
                .split(',')
                .map(|s| format!("html[data-theme=\"dark\"] {}", s.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&scoped);
            out.push_str(" {\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn get_highlighter() -> &'static Highlighter {
    HIGHLIGHTER.get_or_init(|| {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        let light_theme = theme_set
            .themes
            .get("InspiredGitHub")
            .or_else(|| theme_set.themes.values().next())
            .expect("syntect default themes should not be empty");
        let light_css = css_for_theme_with_class_style(light_theme, CLASS_STYLE)
            .unwrap_or_default();

        let dark_theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.get("Solarized (dark)"))
            .unwrap_or(light_theme);
        let dark_raw = css_for_theme_with_class_style(dark_theme, CLASS_STYLE)
            .unwrap_or_default();
        let dark_css = scope_dark_syntax_css(&dark_raw);

        let css = format!("{light_css}\n{dark_css}");
        Highlighter { syntax_set, css }
    })
}

/// Pre-warms the syntax set and theme CSS in the background so the first notebook opens instantly.
pub fn warm_up() {
    let _ = get_highlighter();
}

/// Returns the generated CSS stylesheet for the `InspiredGitHub` light theme.
pub fn syntax_css() -> &'static str {
    &get_highlighter().css
}

/// Escapes special HTML characters in plain text.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn normalize_lang(lang: &str) -> &str {
    let token = lang
        .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match token.as_str() {
        "python" | "python3" | "py" | "ipython" | "ipython3" | "sage" => "py",
        "r" => "R",
        "julia" | "jl" => "jl",
        "rust" | "rs" => "rs",
        "bash" | "sh" | "shell" | "zsh" | "shell-session" | "console" => "sh",
        "javascript" | "js" | "node" => "js",
        "typescript" | "ts" => "ts",
        "json" | "jsonl" | "geojson" => "json",
        "sql" | "sqlite" | "postgres" | "postgresql" | "mysql" => "sql",
        "c" => "c",
        "cpp" | "c++" | "cxx" | "cc" => "cpp",
        "html" | "htm" | "xml" => "html",
        "css" | "scss" => "css",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "markdown" | "md" => "md",
        "latex" | "tex" => "tex",
        "java" => "java",
        "go" | "golang" => "go",
        "ruby" | "rb" => "rb",
        "php" => "php",
        "scala" => "scala",
        "diff" | "patch" => "diff",
        "makefile" | "make" => "Makefile",
        "dockerfile" | "docker" => "Dockerfile",
        _ => "",
    }
}

fn find_syntax<'a>(ss: &'a SyntaxSet, code: &str, lang: &str) -> Option<&'a SyntaxReference> {
    let first_line = code.lines().next().unwrap_or("").trim();
    if let Some(magic) = first_line.strip_prefix("%%") {
        let magic_name = magic.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        let magic_lang = match magic_name.as_str() {
            "bash" | "sh" | "script" => Some("sh"),
            "html" => Some("html"),
            "sql" => Some("sql"),
            "javascript" | "js" => Some("js"),
            "latex" => Some("tex"),
            "markdown" => Some("md"),
            "ruby" => Some("rb"),
            "r" => Some("R"),
            _ => None,
        };
        if let Some(ext) = magic_lang {
            if let Some(syn) = ss.find_syntax_by_token(ext) {
                return Some(syn);
            }
        }
    }

    let mapped = normalize_lang(lang);
    if !mapped.is_empty() {
        if let Some(syn) = ss.find_syntax_by_token(mapped) {
            return Some(syn);
        }
    }
    if !lang.trim().is_empty() {
        let raw = lang.trim();
        if let Some(syn) = ss.find_syntax_by_token(raw) {
            return Some(syn);
        }
        if let Some(syn) = ss.find_syntax_by_name(raw) {
            return Some(syn);
        }
    }
    None
}

/// Highlights `code` using `syntect` classed HTML spans (`syn-*`).
/// Falls back to HTML-escaped plain text for unknown languages or very large inputs.
pub fn highlight_code(code: &str, lang: &str) -> String {
    if code.is_empty() {
        return String::new();
    }
    if code.len() > MAX_HIGHLIGHT_BYTES {
        return html_escape(code);
    }
    let hl = get_highlighter();
    let Some(syntax) = find_syntax(&hl.syntax_set, code, lang) else {
        return html_escape(code);
    };

    let mut gen = ClassedHTMLGenerator::new_with_class_style(syntax, &hl.syntax_set, CLASS_STYLE);
    for line in LinesWithEndings::from(code) {
        if gen.parse_html_for_line_which_includes_newline(line).is_err() {
            return html_escape(code);
        }
    }
    gen.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_python_with_classes() {
        let html = highlight_code("def add(x, y):\n    return x + y\n", "python");
        assert!(html.contains("syn-"));
        assert!(html.contains("def"));
    }

    #[test]
    fn handles_cell_magic() {
        let html = highlight_code("%%bash\necho \"hello\"\n", "python");
        assert!(html.contains("syn-"));
    }

    #[test]
    fn generates_light_and_dark_css() {
        let css = syntax_css();
        assert!(css.contains(".syn-"));
        assert!(css.contains("html[data-theme=\"dark\"] .syn-"));
    }
}
