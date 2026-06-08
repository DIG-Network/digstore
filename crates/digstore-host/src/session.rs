//! Attestation-gated session state (§12). A session is established after a
//! successful attestation handshake and gates `jwks_fetch` (§6.3).

#[derive(Debug, Clone, Copy)]
pub struct Session {
    pub nonce: [u8; 32],
    pub store_id: [u8; 32],
    pub established_at: u64,
    pub expires_at: u64,
}

impl Session {
    pub fn is_valid_at(&self, now: u64) -> bool {
        now < self.expires_at
    }
}

#[derive(Debug, Default)]
pub struct SessionTable {
    current: Option<Session>,
}

impl SessionTable {
    pub fn new() -> Self {
        SessionTable { current: None }
    }

    /// Establish (or replace) the active session with a TTL in seconds.
    pub fn establish(&mut self, nonce: [u8; 32], store_id: [u8; 32], now: u64, ttl_secs: u64) {
        self.current = Some(Session {
            nonce,
            store_id,
            established_at: now,
            expires_at: now.saturating_add(ttl_secs),
        });
    }

    pub fn is_valid(&self, now: u64) -> bool {
        self.current.map(|s| s.is_valid_at(now)).unwrap_or(false)
    }

    pub fn active_store_id(&self, now: u64) -> Option<[u8; 32]> {
        self.current
            .filter(|s| s.is_valid_at(now))
            .map(|s| s.store_id)
    }

    pub fn clear(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_session_is_invalid() {
        let table = SessionTable::new();
        assert!(!table.is_valid(100));
    }

    #[test]
    fn established_session_is_valid_before_expiry() {
        let mut table = SessionTable::new();
        table.establish([9u8; 32], [3u8; 32], 100, 60);
        assert!(table.is_valid(120));
        assert_eq!(table.active_store_id(120), Some([3u8; 32]));
    }

    #[test]
    fn session_expires_after_ttl() {
        let mut table = SessionTable::new();
        table.establish([9u8; 32], [3u8; 32], 100, 60);
        assert!(!table.is_valid(161));
        assert_eq!(table.active_store_id(161), None);
    }

    #[test]
    fn clear_removes_session() {
        let mut table = SessionTable::new();
        table.establish([9u8; 32], [3u8; 32], 100, 60);
        table.clear();
        assert!(!table.is_valid(120));
    }
}
