//! Loading / saving projects (.smp = TOML).

use smk_core::project::Project;
use std::path::{Path, PathBuf};

pub fn load(path: &Path) -> Result<Project, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    toml::from_str(&s).map_err(|e| format!("{}: {}", path.display(), e))
}

pub fn save(p: &Project, path: &Path) -> Result<(), String> {
    let s = toml::to_string_pretty(p).map_err(|e| e.to_string())?;
    let header = format!("# Setup Maker project (v{})\n# Документация: README.md. Пути — относительно этого файла.\n\n", smk_core::VERSION);
    std::fs::write(path, header + &s).map_err(|e| format!("{}: {}", path.display(), e))
}

/// Load `.smp`. Returns (project, base dir).
pub fn load_with_dir(path: &Path) -> Result<(Project, PathBuf), String> {
    let dir = path.parent().map(|p| p.to_path_buf()).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from("."));
    let dir = dir.canonicalize().map(strip_verbatim).unwrap_or(dir);
    Ok((load(path)?, dir))
}

/// `\\?\C:\x` -> `C:\x` (canonicalize on Windows returns verbatim paths).
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}
