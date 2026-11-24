# eojupyter

A fast, read-only, light-mode IDE-style Jupyter notebook (`.ipynb`) viewer written in Rust.

- **Left Pane**: VS Code-style file explorer with instant keyboard navigation (`↑` / `↓` / `←` / `→`), `.ipynb` vs `All` filter toggle, and fuzzy/substring search (`/` or `Ctrl+P`).
- **Right Pane**: GitHub / Jupyter Notebook-style read-only viewer (execution disabled), defaulting to crisp Light mode with an instant Light/Dark toggle (`t`). Supports nbformat v3 & v4, syntax highlighting (`InspiredGitHub` light / `base16-ocean.dark`), GFM Markdown + MathML equations, pandas DataFrame HTML tables, PNG/JPEG/GIF/WebP/SVG outputs, cell attachments, ANSI colors, `\r` progress bar normalization, and XSS sanitization.
- **Fast**: Native Rust parsing + LRU doc/blob cache + neighbor prefetching + instant `PgUp` / `PgDn` scrolling without changing pane focus.

## Build

```bash
cargo build --release
```

The binary is placed at `target/release/eojupyter`.

## Usage

```bash
# Open current directory in native desktop window
./target/release/eojupyter

# Open a specific directory or notebook file
./target/release/eojupyter ./examples
./target/release/eojupyter ./examples/01_overview.ipynb

# Serve over local HTTP and open in your browser instead of a native window
./target/release/eojupyter ./examples --web
./target/release/eojupyter ./examples --web --port 8787 --no-open
```

## Keyboard Shortcuts

| Key | Action |
| --- | --- |
| `↑` / `↓` or `j` / `k` | Select previous / next file or folder in explorer (auto-opens files) |
| `→` / `l` | Expand folder or step into first child |
| `←` / `h` | Collapse folder or jump to parent folder |
| `PgDn` / `PgUp` | Scroll notebook down / up (works anytime without clicking right pane) |
| `Space` / `Shift+Space` | Page down / up in notebook |
| `Home` / `End` | Jump to top / bottom of notebook |
| `]` / `[` or `n` / `p` | Jump to next / previous cell |
| `/` or `Ctrl+P` / `Ctrl+F` | Focus quick file search (`Esc` to clear) |
| `t` | Toggle Light / Dark theme |
| `Ctrl+B` | Toggle file explorer sidebar |
| `r` or `F5` | Refresh directory tree and force-reload active notebook |
| `Esc` | Close zoomed image modal or search input |
