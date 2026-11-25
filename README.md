# <img src="icons/eojupyter.svg" width="36" height="36" align="center" alt="eojupyter icon"> eojupyter — Eye of Jupyter

A fast, read-only, IDE-style Jupyter Notebook (`.ipynb`) viewer written in Rust.

Browse directories in a VS Code-style file explorer on the left and read rendered notebooks on the right—like GitHub's notebook view, without spinning up a Jupyter kernel or allowing code execution.

## Features

- **Two-Pane IDE Layout**: Resizable file explorer sidebar on the left (with `.ipynb` / `+.py` / `All` filter switch, workspace root switcher, and quick filter) paired with a clean Jupyter Notebook viewer on the right.
- **Strict Read-Only Mode**: No kernel execution; Markdown and `text/html` outputs are sanitized with `ammonia`, and SVG outputs are served sandboxed via `<img>`.
- **Light Mode Default + Dark Mode Toggle**: Opens in crisp Light mode (`InspiredGitHub` syntax theme) with an instant Light/Dark toggle (`t` or toolbar button, using `base16-ocean.dark` in dark mode, persisted in `localStorage`).
- **Fast Native Rust Engine**:
  - Supports `nbformat` v3 (`worksheets`) and v4 notebooks.
  - GFM Markdown (`pulldown-cmark`) with native MathML equations (`latex2mathml`).
  - Rich cell outputs: pandas `DataFrame` HTML tables, PNG/JPEG/GIF/WebP/SVG images, cell `attachment:` images, LaTeX, JSON, and ANSI color + `\r` progress-bar normalization.
  - Also previews `.md`, image, and syntax-highlighted text/code files when viewing in `All` mode.
  - Thread-safe LRU document/blob cache (invalidated by file mtime/size), `AbortController` request cancellation, and neighbor prefetching for instant file switching.
- **Seamless Keyboard Workflow**: Navigate the file tree with arrow keys (`↑` / `↓` / `←` / `→`) while scrolling the notebook with `PgUp` / `PgDn`—no need to click between panes to switch focus.
- **Desktop Window or Browser Mode**: Launches a native desktop window (`tao` + `wry` / WebKitGTK), or serves over local HTTP with `--web`.

## Requirements

- Rust toolchain (`rustc` and `cargo`)
- On Linux for native desktop window mode: `gtk+-3.0` and `webkit2gtk-4.1` development libraries (`pkg-config`)

## Build & Install

```bash
# Build release binary
cargo build --release

# Run tests
cargo test

# Optional: install onto PATH (~/.cargo/bin)
cargo install --path .

# Optional (Linux): install desktop launcher & application icons (Wayland/X11 dock)
./scripts/install-desktop.sh

# Optional (Debian/Ubuntu): build a .deb package and install it system-wide
./scripts/build-deb.sh
sudo apt install ./dist/eojupyter_0.1.0_amd64.deb
```

The compiled release binary is at `target/release/eojupyter`.

## Usage

```bash
# Open current working directory in a native desktop window
eojupyter

# Open a specific directory or notebook file
eojupyter ./examples
eojupyter ./examples/01_overview.ipynb

# Serve over local HTTP and open in your default browser
eojupyter ./examples --web

# Serve on a specific port without launching a browser
eojupyter ./examples --web --port 8787 --no-open
```

### CLI Options

| Flag / Argument | Description |
| --- | --- |
| `[PATH]` | Directory or `.ipynb` file to open (defaults to current working directory) |
| `--web` | Serve over local HTTP instead of launching a native desktop window |
| `--port <PORT>` | Port to bind for the internal HTTP server (`0` = automatic) |
| `--no-open` | In `--web` mode, print the URL without launching a browser |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

## Keyboard Shortcuts

| Key | Action |
| --- | --- |
| `↑` / `↓` or `j` / `k` | Select previous / next entry in file explorer (auto-renders files) |
| `→` / `l` | Expand folder or move to its first child |
| `←` / `h` | Collapse folder or jump to parent folder |
| `Enter` | Toggle folder expand/collapse or reload selected file |
| `PgDn` / `PgUp` | Scroll notebook down / up (works anytime without changing focus) |
| `Space` / `Shift+Space` | Scroll notebook page down / up |
| `Home` / `End` | Jump to top / bottom of notebook |
| `]` / `[` or `n` / `p` | Jump to next / previous cell |
| `/` or `Ctrl+P` / `Ctrl+F` | Focus quick file filter (`Esc` to clear and blur) |
| `f` | Cycle explorer filter mode (`.ipynb` → `+.py` → `All`) |
| `t` | Toggle Light / Dark theme |
| `Ctrl+B` | Toggle file explorer sidebar |
| `r` or `F5` | Refresh directory tree and force-reload active file |
| `Esc` | Close zoomed image modal, clear filter, or cancel root path edit |

## Project Structure

```text
eojupyter/
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs        # CLI flags, native tao+wry window & --web fallback
│   ├── server.rs      # Multithreaded tiny_http server & JSON/blob API routes
│   ├── fs_tree.rs     # Traversal-safe directory listing & recursive search
│   ├── notebook.rs    # nbformat v3/v4 parser, Markdown+MathML, outputs, sanitization
│   ├── highlight.rs   # Syntect highlighting (InspiredGitHub / base16-ocean.dark)
│   ├── ansi.rs        # ANSI SGR & carriage-return (\r) terminal output renderer
│   └── cache.rs       # Thread-safe LRU document & binary blob cache
├── web/
│   ├── index.html     # Split-pane IDE shell
│   ├── style.css      # Light (default) & Dark mode styles
│   └── app.js         # Tree state, global keyboard navigation, prefetching
└── examples/          # Sample notebooks for quick testing
```
