use serde::Serialize;
use std::collections::VecDeque;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub ext: String,
    pub size: u64,
    pub is_notebook: bool,
}

const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".ipynb_checkpoints",
    "__pycache__",
    "node_modules",
    "target",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
];

fn is_ignored_name(name: &str, is_dir: bool, show_hidden: bool) -> bool {
    if !show_hidden && name.starts_with('.') {
        return true;
    }
    if is_dir && !show_hidden && IGNORED_DIRS.contains(&name) {
        return true;
    }
    false
}

/// Converts a PathBuf relative to `root` into a clean forward-slash string.
pub fn to_rel_posix(root: &Path, target: &Path) -> String {
    match target.strip_prefix(root) {
        Ok(rel) => rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => String::new(),
    }
}

/// Resolves a relative path safely inside `root`, rejecting traversal outside `root`.
pub fn resolve_safe_path(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let trimmed = rel_path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(root.to_path_buf());
    }
    let mut candidate = root.to_path_buf();
    for comp in Path::new(trimmed).components() {
        match comp {
            Component::Normal(seg) => candidate.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("Invalid path traversal".to_string());
            }
        }
    }
    let canonical = fs::canonicalize(&candidate).map_err(|e| format!("Not found: {e}"))?;
    if !canonical.starts_with(root) {
        return Err("Access outside root directory denied".to_string());
    }
    Ok(canonical)
}

/// Resolves a relative link found inside a notebook or markdown file located at `doc_rel_path`.
pub fn resolve_relative_asset(
    root: &Path,
    doc_rel_path: &str,
    asset_link: &str,
) -> Option<String> {
    let clean_link = asset_link
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .trim();
    if clean_link.is_empty() {
        return None;
    }
    let doc_dir = Path::new(doc_rel_path).parent().unwrap_or_else(|| Path::new(""));
    let joined = root.join(doc_dir).join(clean_link);
    let canonical = fs::canonicalize(joined).ok()?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return None;
    }
    Some(to_rel_posix(root, &canonical))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    NotebooksOnly,
    NotebooksAndPy,
    All,
}

impl FilterMode {
    pub fn from_str_opt(mode: Option<&str>, notebooks_only_fallback: bool) -> Self {
        match mode.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("ipynb") => Self::NotebooksOnly,
            Some("ipynb_py") | Some("py") => Self::NotebooksAndPy,
            Some("all") => Self::All,
            _ => {
                if notebooks_only_fallback {
                    Self::NotebooksOnly
                } else {
                    Self::All
                }
            }
        }
    }

    pub fn matches_ext(self, ext: &str) -> bool {
        match self {
            Self::NotebooksOnly => ext == "ipynb",
            Self::NotebooksAndPy => ext == "ipynb" || ext == "py",
            Self::All => true,
        }
    }
}

fn dir_contains_matching(dir: &Path, mode: FilterMode, depth: usize) -> bool {
    if mode == FilterMode::All {
        return true;
    }
    if depth > 4 {
        return true;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let is_dir = ft.is_dir();
        if is_ignored_name(&name, is_dir, false) {
            continue;
        }
        if !is_dir {
            let ext = Path::new(&name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if mode.matches_ext(&ext) {
                return true;
            }
        } else {
            subdirs.push(entry.path());
        }
    }
    subdirs
        .into_iter()
        .take(32)
        .any(|sub| dir_contains_matching(&sub, mode, depth + 1))
}

pub fn list_dir(
    root: &Path,
    rel_path: &str,
    mode: FilterMode,
    show_hidden: bool,
) -> Result<Vec<TreeEntry>, String> {
    let target = resolve_safe_path(root, rel_path)?;
    if !target.is_dir() {
        return Err("Target path is not a directory".to_string());
    }

    let read_dir = fs::read_dir(&target).map_err(|e| format!("Failed to read directory: {e}"))?;
    let mut items = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let is_dir = meta.is_dir();
        if is_ignored_name(&name, is_dir, show_hidden) {
            continue;
        }

        let ext = if is_dir {
            String::new()
        } else {
            path.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        };
        let is_notebook = !is_dir && ext == "ipynb";

        if !is_dir && !mode.matches_ext(&ext) {
            continue;
        }
        if is_dir && mode != FilterMode::All && !dir_contains_matching(&path, mode, 0) {
            continue;
        }

        let rel = to_rel_posix(root, &path);
        if rel.is_empty() {
            continue;
        }

        items.push(TreeEntry {
            name,
            path: rel,
            is_dir,
            ext,
            size: if is_dir { 0 } else { meta.len() },
            is_notebook,
        });
    }

    items.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(items)
}

pub fn search_files(
    root: &Path,
    query: &str,
    mode: FilterMode,
    show_hidden: bool,
    limit: usize,
) -> Vec<TreeEntry> {
    let q = query.trim().to_ascii_lowercase();
    let mut results = Vec::new();
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        if depth > 8 || results.len() >= limit {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();

        for path in children {
            if results.len() >= limit {
                break;
            }
            let Some(name_os) = path.file_name() else {
                continue;
            };
            let name = name_os.to_string_lossy().to_string();
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            let is_dir = meta.is_dir();
            if is_ignored_name(&name, is_dir, show_hidden) {
                continue;
            }
            let ext = if is_dir {
                String::new()
            } else {
                path.extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
            };
            let is_notebook = !is_dir && ext == "ipynb";
            let rel = to_rel_posix(root, &path);

            let matches_query = q.is_empty() || rel.to_ascii_lowercase().contains(&q);
            if matches_query && mode.matches_ext(&ext) && !is_dir {
                results.push(TreeEntry {
                    name,
                    path: rel,
                    is_dir,
                    ext,
                    size: meta.len(),
                    is_notebook,
                });
            }

            if is_dir && depth < 8 {
                queue.push_back((path, depth + 1));
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("eojupyter_test_{label}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_dirs_first_and_filters_notebooks() {
        let root = make_temp_dir("list");
        let sub = root.join("alpha");
        let py_sub = root.join("beta_py");
        let empty_sub = root.join("empty_dir");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir_all(&py_sub).unwrap();
        fs::create_dir_all(&empty_sub).unwrap();
        fs::write(sub.join("inner.ipynb"), "{}").unwrap();
        fs::write(py_sub.join("util.py"), "x = 1").unwrap();
        fs::write(root.join("notes.md"), "# hi").unwrap();
        fs::write(root.join("helper.py"), "def f(): pass").unwrap();
        fs::write(root.join("main.ipynb"), "{}").unwrap();

        let nb_only = list_dir(&root, "", FilterMode::NotebooksOnly, false).unwrap();
        let names: Vec<_> = nb_only.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "main.ipynb"]);

        let nb_and_py = list_dir(&root, "", FilterMode::NotebooksAndPy, false).unwrap();
        let py_names: Vec<_> = nb_and_py.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(py_names, vec!["alpha", "beta_py", "helper.py", "main.ipynb"]);

        let all = list_dir(&root, "", FilterMode::All, false).unwrap();
        let all_names: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            all_names,
            vec!["alpha", "beta_py", "empty_dir", "helper.py", "main.ipynb", "notes.md"]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_path_traversal() {
        let root = make_temp_dir("trav");
        assert!(resolve_safe_path(&root, "../etc/passwd").is_err());
        assert!(resolve_safe_path(&root, "/../../etc").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
