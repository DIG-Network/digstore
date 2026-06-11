//! Small filesystem helpers shared within the crate.

use std::path::Path;

/// Writes bytes to `path` with owner-only permissions where the platform
/// supports it (unix `0600`); on Windows the file inherits the (already
/// user-scoped) parent directory ACL. Creates the parent directory if needed.
pub(crate) fn write_secret_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)?;
        f.flush()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}
