use crate::highlight::html_escape;
use std::fmt::Write as _;

#[derive(Clone, Default, PartialEq, Eq)]
struct AnsiState {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    fg: Option<AnsiColor>,
    bg: Option<AnsiColor>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnsiColor {
    Class(&'static str),
    Rgb(u8, u8, u8),
}

impl AnsiState {
    fn is_default(&self) -> bool {
        !self.bold
            && !self.dim
            && !self.italic
            && !self.underline
            && self.fg.is_none()
            && self.bg.is_none()
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn write_open_span(&self, out: &mut String) {
        out.push_str("<span");
        let mut classes: Vec<&'static str> = Vec::new();
        if self.bold {
            classes.push("ansi-bold");
        }
        if self.dim {
            classes.push("ansi-dim");
        }
        if self.italic {
            classes.push("ansi-italic");
        }
        if self.underline {
            classes.push("ansi-underline");
        }
        if let Some(AnsiColor::Class(c)) = self.fg {
            classes.push(c);
        }
        if let Some(AnsiColor::Class(c)) = self.bg {
            classes.push(c);
        }
        if !classes.is_empty() {
            out.push_str(" class=\"");
            for (i, c) in classes.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(c);
            }
            out.push('"');
        }
        let has_rgb_fg = matches!(self.fg, Some(AnsiColor::Rgb(..)));
        let has_rgb_bg = matches!(self.bg, Some(AnsiColor::Rgb(..)));
        if has_rgb_fg || has_rgb_bg {
            out.push_str(" style=\"");
            if let Some(AnsiColor::Rgb(r, g, b)) = self.fg {
                let _ = write!(out, "color:rgb({},{},{});", r, g, b);
            }
            if let Some(AnsiColor::Rgb(r, g, b)) = self.bg {
                let _ = write!(out, "background-color:rgb({},{},{});", r, g, b);
            }
            out.push('"');
        }
        out.push('>');
    }
}

fn fg_class(code: u16) -> Option<&'static str> {
    match code {
        30 => Some("ansi-fg-black"),
        31 => Some("ansi-fg-red"),
        32 => Some("ansi-fg-green"),
        33 => Some("ansi-fg-yellow"),
        34 => Some("ansi-fg-blue"),
        35 => Some("ansi-fg-magenta"),
        36 => Some("ansi-fg-cyan"),
        37 => Some("ansi-fg-white"),
        90 => Some("ansi-fg-bright-black"),
        91 => Some("ansi-fg-bright-red"),
        92 => Some("ansi-fg-bright-green"),
        93 => Some("ansi-fg-bright-yellow"),
        94 => Some("ansi-fg-bright-blue"),
        95 => Some("ansi-fg-bright-magenta"),
        96 => Some("ansi-fg-bright-cyan"),
        97 => Some("ansi-fg-bright-white"),
        _ => None,
    }
}

fn bg_class(code: u16) -> Option<&'static str> {
    match code {
        40 => Some("ansi-bg-black"),
        41 => Some("ansi-bg-red"),
        42 => Some("ansi-bg-green"),
        43 => Some("ansi-bg-yellow"),
        44 => Some("ansi-bg-blue"),
        45 => Some("ansi-bg-magenta"),
        46 => Some("ansi-bg-cyan"),
        47 => Some("ansi-bg-white"),
        100 => Some("ansi-bg-bright-black"),
        101 => Some("ansi-bg-bright-red"),
        102 => Some("ansi-bg-bright-green"),
        103 => Some("ansi-bg-bright-yellow"),
        104 => Some("ansi-bg-bright-blue"),
        105 => Some("ansi-bg-bright-magenta"),
        106 => Some("ansi-bg-bright-cyan"),
        107 => Some("ansi-bg-bright-white"),
        _ => None,
    }
}

fn xterm_256_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0 => (0, 0, 0),
        1 => (187, 0, 0),
        2 => (0, 187, 0),
        3 => (187, 187, 0),
        4 => (0, 0, 187),
        5 => (187, 0, 187),
        6 => (0, 187, 187),
        7 => (187, 187, 187),
        8 => (85, 85, 85),
        9 => (255, 85, 85),
        10 => (85, 255, 85),
        11 => (255, 255, 85),
        12 => (85, 85, 255),
        13 => (255, 85, 255),
        14 => (85, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let n = idx - 16;
            let r = n / 36;
            let g = (n % 36) / 6;
            let b = n % 6;
            let conv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (conv(r), conv(g), conv(b))
        }
        232..=255 => {
            let v = 8 + (idx - 232) * 10;
            (v, v, v)
        }
    }
}

/// Applies terminal carriage-return (`\r`) overwrite semantics per line (e.g., tqdm bars).
pub fn normalize_carriage_returns(input: &str) -> String {
    if !input.contains('\r') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    for segment in input.split_inclusive('\n') {
        let (line, has_nl) = match segment.strip_suffix('\n') {
            Some(l) => (l, true),
            None => (segment, false),
        };
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(idx) = line.rfind('\r') {
            out.push_str(&line[idx + 1..]);
        } else {
            out.push_str(line);
        }
        if has_nl {
            out.push('\n');
        }
    }
    out
}

fn apply_sgr_params(params_str: &str, state: &mut AnsiState) {
    if params_str.is_empty() {
        state.reset();
        return;
    }
    let codes: Vec<u16> = params_str
        .split(';')
        .map(|s| s.parse::<u16>().unwrap_or(0))
        .collect();
    let mut i = 0;
    while i < codes.len() {
        match codes[i] {
            0 => state.reset(),
            1 => state.bold = true,
            2 => state.dim = true,
            3 => state.italic = true,
            4 => state.underline = true,
            22 => {
                state.bold = false;
                state.dim = false;
            }
            23 => state.italic = false,
            24 => state.underline = false,
            39 => state.fg = None,
            49 => state.bg = None,
            38 => {
                if i + 2 < codes.len() && codes[i + 1] == 5 {
                    let (r, g, b) = xterm_256_to_rgb(codes[i + 2].min(255) as u8);
                    state.fg = Some(AnsiColor::Rgb(r, g, b));
                    i += 2;
                } else if i + 4 < codes.len() && codes[i + 1] == 2 {
                    state.fg = Some(AnsiColor::Rgb(
                        codes[i + 2].min(255) as u8,
                        codes[i + 3].min(255) as u8,
                        codes[i + 4].min(255) as u8,
                    ));
                    i += 4;
                }
            }
            48 => {
                if i + 2 < codes.len() && codes[i + 1] == 5 {
                    let (r, g, b) = xterm_256_to_rgb(codes[i + 2].min(255) as u8);
                    state.bg = Some(AnsiColor::Rgb(r, g, b));
                    i += 2;
                } else if i + 4 < codes.len() && codes[i + 1] == 2 {
                    state.bg = Some(AnsiColor::Rgb(
                        codes[i + 2].min(255) as u8,
                        codes[i + 3].min(255) as u8,
                        codes[i + 4].min(255) as u8,
                    ));
                    i += 4;
                }
            }
            c => {
                if let Some(cls) = fg_class(c) {
                    state.fg = Some(AnsiColor::Class(cls));
                } else if let Some(cls) = bg_class(c) {
                    state.bg = Some(AnsiColor::Class(cls));
                }
            }
        }
        i += 1;
    }
}

/// Converts terminal text with ANSI escape codes and `\r` overwrites into safe HTML.
pub fn ansi_to_html(raw: &str) -> String {
    let text = normalize_carriage_returns(raw);
    if !text.contains('\x1b') {
        return html_escape(&text);
    }

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 32);
    let mut state = AnsiState::default();
    let mut span_open = false;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                // CSI sequence
                let start = i + 2;
                let mut end = start;
                while end < bytes.len() && !(0x40..=0x7e).contains(&bytes[end]) {
                    end += 1;
                }
                if end < bytes.len() {
                    let final_byte = bytes[end];
                    if final_byte == b'm' {
                        let params = &text[start..end];
                        let prev_state = state.clone();
                        apply_sgr_params(params, &mut state);
                        if state != prev_state {
                            if span_open {
                                out.push_str("</span>");
                                span_open = false;
                            }
                            if !state.is_default() {
                                state.write_open_span(&mut out);
                                span_open = true;
                            }
                        }
                    }
                    i = end + 1;
                    continue;
                }
            }
            // Skip lone ESC or unrecognized sequence safely
            i += 1;
            continue;
        }

        // Scan plain chunk up to next ESC
        let plain_start = i;
        while i < bytes.len() && bytes[i] != 0x1b {
            i += 1;
        }
        out.push_str(&html_escape(&text[plain_start..i]));
    }

    if span_open {
        out.push_str("</span>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_sgr_colors_and_escapes_html() {
        let input = "\x1b[1;31mError <tag>\x1b[0m ok";
        let html = ansi_to_html(input);
        assert_eq!(
            html,
            "<span class=\"ansi-bold ansi-fg-red\">Error &lt;tag&gt;</span> ok"
        );
    }

    #[test]
    fn normalizes_carriage_return_progress_bars() {
        let input = "10% [..]\r50% [####]\r100% [########]\nDone\n";
        assert_eq!(
            normalize_carriage_returns(input),
            "100% [########]\nDone\n"
        );
    }

    #[test]
    fn supports_256_and_rgb_colors() {
        let input = "\x1b[38;5;196mhi\x1b[0m \x1b[38;2;10;20;30mrgb\x1b[0m";
        let html = ansi_to_html(input);
        assert!(html.contains("color:rgb("));
        assert!(html.contains("color:rgb(10,20,30);"));
    }
}
