pub mod beacon;
pub mod branding;
pub mod cli;
pub mod commands;
pub mod config;
pub mod context;
pub mod dig_toml;
pub mod error;
pub mod ops;
pub mod output;
pub mod runtime;
pub mod templates;
pub mod ui;
pub mod workspace;

/// Test-only helpers shared across `src/**` unit-test modules.
#[cfg(test)]
pub(crate) mod testutil {
    use std::sync::Mutex;

    /// ONE process-wide lock for every test that mutates the process-global
    /// `DIG_IDENTITY_DIR` env var (and its cousins read from the same dir,
    /// `DIG_NODE_URL`/`DIG_NODE_PORT`). This var is read by `ops::identity`,
    /// `ops::dighub`, and `ops::node` across SEPARATE test modules that all
    /// run in the SAME process (lib unit tests share one binary and run in
    /// parallel by default) — a per-module `Mutex` does NOT serialize across
    /// modules, since each one only guards itself against ITS OWN other
    /// tests. Every test that sets/removes `DIG_IDENTITY_DIR` (or a var read
    /// alongside it in the same test) MUST hold this lock for its entire body
    /// (set -> assert -> restore), otherwise a concurrent test in another
    /// module can observe a mid-mutation value and fail non-deterministically
    /// (this is exactly how `ops::identity`'s `identity_is_created_then_stable`
    /// was observed to flake against `ops::node`'s tests before this lock was
    /// unified).
    pub(crate) static DIG_IDENTITY_DIR_ENV_LOCK: Mutex<()> = Mutex::new(());
}
