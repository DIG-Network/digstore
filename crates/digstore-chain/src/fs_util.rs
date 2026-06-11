//! Small filesystem helpers shared within the crate.

use std::path::Path;

/// Writes bytes to `path` with owner-only permissions where the platform
/// supports it (unix `0600`); on Windows the file inherits the (already
/// user-scoped) parent directory ACL. Creates the parent directory if needed.
///
/// The write is atomic: bytes are written to a temp file in the same directory
/// and then renamed over the target, so an interrupted write can never corrupt
/// or destroy an existing secret file (e.g. `seed.enc`).
pub(crate) fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    // Temp file in the SAME directory so the rename is atomic (same filesystem).
    let tmp = {
        let mut name = path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        name.push(".tmp");
        dir.join(name)
    };

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&tmp, bytes)?;
    }

    // Atomic replace. std::fs::rename uses MoveFileEx(REPLACE_EXISTING) on Windows
    // and rename(2) on unix, both of which replace an existing target atomically.
    std::fs::rename(&tmp, path)?;
    Ok(())
}
