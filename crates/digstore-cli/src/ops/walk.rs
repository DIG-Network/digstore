//! Resolve `add` path arguments into a concrete list of files + their resource
//! keys, honoring `.digignore`/`.gitignore` and always skipping the `.dig/` store
//! directory. Keys are the file path relative to the store root, forward-slashed.

use std::path::{Path, PathBuf};

use globset::Glob;
use ignore::WalkBuilder;

/// A resolved file to stage: absolute path + portable resource key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub key: String,
}

/// Resource key = `path` relative to `root`, forward-slashed.
fn key_for(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Walk `dir` (under `root`) collecting non-ignored files (skips `.dig/`).
fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<Resolved>) {
    let mut wb = WalkBuilder::new(dir);
    wb.hidden(false) // include dotfiles (Git stages them unless ignored)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .add_custom_ignore_filename(".digignore");
    // Always skip the store directory.
    let store_dir = root.join(".dig");
    for entry in wb.build().flatten() {
        let p = entry.path();
        if p.starts_with(&store_dir) {
            continue;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            out.push(Resolved {
                path: p.to_path_buf(),
                key: key_for(root, p),
            });
        }
    }
}

/// Resolve one argument (file, directory, or glob) relative to `root`.
pub fn resolve_arg(root: &Path, arg: &str, out: &mut Vec<Resolved>) -> Result<(), String> {
    let as_path = root.join(arg);
    if as_path.is_file() {
        out.push(Resolved {
            path: as_path.clone(),
            key: key_for(root, &as_path),
        });
        return Ok(());
    }
    if as_path.is_dir() {
        walk_dir(root, &as_path, out);
        return Ok(());
    }
    // Treat as a glob relative to root.
    let glob = Glob::new(arg)
        .map_err(|e| format!("bad pattern '{arg}': {e}"))?
        .compile_matcher();
    let mut all = Vec::new();
    walk_dir(root, root, &mut all);
    let before = out.len();
    for r in all {
        if glob.is_match(&r.key) {
            out.push(r);
        }
    }
    if out.len() == before {
        return Err(format!("no files matched '{arg}'"));
    }
    Ok(())
}

/// Resolve `--all`: every non-ignored file under the store root.
pub fn resolve_all(root: &Path) -> Vec<Resolved> {
    let mut out = Vec::new();
    walk_dir(root, root, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join(".dig/modules")).unwrap();
        fs::write(d.path().join("a.txt"), b"a").unwrap();
        fs::create_dir_all(d.path().join("sub")).unwrap();
        fs::write(d.path().join("sub/b.md"), b"b").unwrap();
        fs::write(d.path().join("c.log"), b"c").unwrap();
        fs::write(d.path().join(".digignore"), "*.log\n").unwrap();
        d
    }

    #[test]
    fn resolve_all_skips_store_and_ignored() {
        let d = scratch();
        let keys: Vec<String> = resolve_all(d.path()).into_iter().map(|r| r.key).collect();
        assert!(keys.contains(&"a.txt".to_string()));
        assert!(keys.contains(&"sub/b.md".to_string()));
        assert!(
            !keys.iter().any(|k| k.contains(".dig/")),
            "store dir skipped"
        );
        assert!(!keys.contains(&"c.log".to_string()), ".digignore honored");
        // .digignore itself may or may not appear depending on the ignore crate's
        // treatment of the custom ignore file; either outcome is acceptable.
    }

    #[test]
    fn resolve_glob_matches_relative_keys() {
        let d = scratch();
        let mut out = Vec::new();
        resolve_arg(d.path(), "sub/*.md", &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "sub/b.md");
    }

    #[test]
    fn resolve_single_file() {
        let d = scratch();
        let mut out = Vec::new();
        resolve_arg(d.path(), "a.txt", &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "a.txt");
    }

    #[test]
    fn glob_with_no_match_errors() {
        let d = scratch();
        let mut out = Vec::new();
        assert!(resolve_arg(d.path(), "*.nope", &mut out).is_err());
    }
}
