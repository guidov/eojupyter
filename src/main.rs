mod ansi;
mod cache;
mod fs_tree;
mod highlight;
mod notebook;
mod server;

use clap::Parser;
use server::{start_server, AppState};
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;

const APP_ICON_PNG: &[u8] = include_bytes!("../icons/eojupyter-128.png");

#[derive(Parser, Debug)]
#[command(
    name = "eojupyter",
    version,
    about = "Fast, read-only, light-mode IDE-style Jupyter notebook viewer"
)]
struct Cli {
    /// Directory or .ipynb file to open (defaults to current working directory)
    #[arg(value_name = "PATH")]
    path: Option<PathBuf>,

    /// Serve over local HTTP instead of launching a native desktop window
    #[arg(long)]
    web: bool,

    /// Port to bind for the internal HTTP server (0 = automatic)
    #[arg(long, default_value_t = 0)]
    port: u16,

    /// In --web mode, print URL without launching a browser
    #[arg(long)]
    no_open: bool,
}

fn resolve_target(path_arg: Option<PathBuf>) -> Result<(PathBuf, Option<String>), String> {
    let target = match path_arg {
        Some(p) => p,
        None => env::current_dir().map_err(|e| format!("Cannot determine cwd: {e}"))?,
    };
    let canonical = fs::canonicalize(&target)
        .map_err(|e| format!("Path not found ({}): {e}", target.display()))?;

    if canonical.is_file() {
        let parent = canonical
            .parent()
            .ok_or_else(|| "File has no parent directory".to_string())?
            .to_path_buf();
        let file_name = canonical
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "Invalid file name".to_string())?
            .to_string();
        Ok((parent, Some(file_name)))
    } else if canonical.is_dir() {
        Ok((canonical, None))
    } else {
        Err(format!("Unsupported path type: {}", canonical.display()))
    }
}

fn has_display_server() -> bool {
    env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
        || env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn run_web_mode(url: &str, no_open: bool) {
    println!("eojupyter running at {url}");
    if !no_open {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
    loop {
        thread::park();
    }
}

fn load_window_icon() -> Option<tao::window::Icon> {
    let mut decoder = png::Decoder::new(Cursor::new(APP_ICON_PNG));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let bytes = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in bytes.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            out
        }
        _ => return None,
    };
    tao::window::Icon::from_rgba(rgba, info.width, info.height).ok()
}

fn run_native_window(url: &str, title: &str) -> Result<(), String> {
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    let event_loop = {
        use tao::platform::unix::EventLoopBuilderExtUnix;
        EventLoopBuilder::new()
            .with_app_id("io.github.guidov.eojupyter")
            .build()
    };

    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let event_loop = EventLoopBuilder::new().build();

    let window = WindowBuilder::new()
        .with_title(title)
        .with_window_icon(load_window_icon())
        .with_inner_size(LogicalSize::new(1400.0, 900.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .build(&event_loop)
        .map_err(|e| format!("Failed to create window: {e}"))?;

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or_else(|| "Window does not expose a GTK vbox container".to_string())?;
        let wv = wry::WebViewBuilder::new()
            .with_url(url)
            .build_gtk(vbox)
            .map_err(|e| format!("Failed to build WebKitGTK webview: {e}"))?;
        let _ = wv.focus();
        wv
    };

    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let _webview = {
        let wv = wry::WebViewBuilder::new()
            .with_url(url)
            .build(&window)
            .map_err(|e| format!("Failed to build webview: {e}"))?;
        let _ = wv.focus();
        wv
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}

fn main() {
    let cli = Cli::parse();
    let (root, initial_file) = match resolve_target(cli.path) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    thread::spawn(highlight::warm_up);

    let state = Arc::new(AppState::new(root.clone(), initial_file));
    let addr = match start_server(state, cli.port) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let url = format!("http://{addr}/");
    if cli.web || !has_display_server() {
        run_web_mode(&url, cli.no_open);
    } else {
        let dir_name = root
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| root.display().to_string());
        let title = format!("eojupyter — {dir_name}");
        if let Err(e) = run_native_window(&url, &title) {
            eprintln!("Native window unavailable ({e}); falling back to web mode.");
            run_web_mode(&url, cli.no_open);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_window_icon_decodes_cleanly() {
        assert!(load_window_icon().is_some());
    }
}
