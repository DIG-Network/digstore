//! Cached-unlock session: stores the decrypted mnemonic in `~/.dig/session`
//! with an absolute expiry, so commands within the TTL skip the passphrase
//! prompt. This trades some security for convenience (an accepted tradeoff);
//! the file is written owner-only and wiped on `lock`/expiry.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

#[derive(Debug, Serialize, Deserialize)]
struct Session {
    /// Absolute expiry, seconds since the unix epoch.
    expires_at: u64,
    phrase: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Caches the phrase with a TTL (seconds from now).
pub fn write_session(path: &Path, phrase: &str, ttl_secs: u64) -> Result<()> {
    let s = Session { expires_at: now_secs().saturating_add(ttl_secs), phrase: phrase.to_string() };
    let json = serde_json::to_vec(&s).map_err(|e| crate::error::ChainError::Config(e.to_string()))?;
    crate::fs_util::write_secret_file(path, &json)?;
    Ok(())
}

/// Reads the cached phrase if present and unexpired; otherwise `None`.
/// An expired session file is removed as a side effect.
pub fn read_session(path: &Path) -> Option<Zeroizing<String>> {
    let bytes = std::fs::read(path).ok()?;
    let s: Session = serde_json::from_slice(&bytes).ok()?;
    if now_secs() >= s.expires_at {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(Zeroizing::new(s.phrase))
}

/// Removes the session file (used by `digstore lock`). Idempotent.
pub fn clear_session(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// True if there is a valid (unexpired) session.
pub fn is_unlocked(path: &Path) -> bool {
    read_session(path).is_some()
}

// Test-only helper to write a session with an absolute expiry.
#[cfg(test)]
fn write_session_abs(path: &Path, phrase: &str, expires_at: u64) -> Result<()> {
    let s = Session { expires_at, phrase: phrase.to_string() };
    let json = serde_json::to_vec(&s).unwrap();
    crate::fs_util::write_secret_file(path, &json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_within_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        write_session(&path, "my phrase", 3600).unwrap();
        assert_eq!(read_session(&path).as_deref().map(|s| s.as_str()), Some("my phrase"));
        assert!(is_unlocked(&path));
    }

    #[test]
    fn expired_session_returns_none_and_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        write_session_abs(&path, "old", 1).unwrap(); // expires_at = 1 (1970)
        assert!(read_session(&path).is_none());
        assert!(!path.exists());
    }

    #[test]
    fn clear_session_removes_file_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session");
        write_session(&path, "x", 3600).unwrap();
        clear_session(&path).unwrap();
        assert!(!path.exists());
        clear_session(&path).unwrap(); // second call: no error
    }
}
