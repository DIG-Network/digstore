//! `digstore doctor` — pre-publish preflight.
//!
//! Publishing is a costly, irreversible on-chain action (the per-capsule $DIG price,
//! plus an XCH fee, per version). `doctor` runs the checks that a publish depends on
//! and prints each as pass/fail, so a developer can fix problems BEFORE spending —
//! not discover them halfway through a paid commit. It reads only; it never spends,
//! anchors, or mutates anything.
//!
//! Checks:
//!   - seed present + unlocked (so a publish can sign),
//!   - wallet funds vs the per-capsule $DIG price + the XCH fee (only when the seed
//!     is unlocked, so `doctor` never prompts for a passphrase),
//!   - dighub login (so `push` to the default remote is authorized),
//!   - the default remote is reachable,
//!   - the content/output directory exists.

use digstore_chain::dig::{self, format_dig, format_xch};
use digstore_chain::{config as chain_config, seed as chain_seed, unlock};

use crate::cli::DoctorArgs;
use crate::context::CliContext;
use crate::dig_toml::DigToml;
use crate::error::CliError;
use crate::runtime::block_on;
use crate::ui::Ui;

/// One preflight check result.
struct Check {
    name: &'static str,
    /// `Some(true)` pass, `Some(false)` fail, `None` skipped/unknown (a soft note).
    status: Option<bool>,
    detail: String,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            status: Some(true),
            detail: detail.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            status: Some(false),
            detail: detail.into(),
        }
    }
    fn skip(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            status: None,
            detail: detail.into(),
        }
    }
}

pub fn run(ctx: &CliContext, ui: &Ui, _args: DoctorArgs) -> Result<(), CliError> {
    let mut checks = Vec::new();

    // 1+2. Seed present + unlocked. A live unlock session means a usable wallet
    //    even without an on-disk `seed.enc` (e.g. a session-only setup), so treat
    //    a present session as "seed available + unlocked".
    let (seed_present, seed_unlocked) = match chain_config::dig_home() {
        Ok(home) => {
            let unlocked = unlock::is_unlocked(&chain_config::session_path(&home));
            let present = unlocked || chain_seed::seed_exists(&chain_config::seed_path(&home));
            if present {
                checks.push(Check::pass("seed", "present"));
            } else {
                checks.push(Check::fail(
                    "seed",
                    "no seed — run `digstore seed import` or `digstore seed generate`",
                ));
            }
            if unlocked {
                checks.push(Check::pass("seed unlocked", "yes"));
            } else if present {
                checks.push(Check::fail(
                    "seed unlocked",
                    "locked — it will prompt for your passphrase at publish",
                ));
            } else {
                checks.push(Check::skip("seed unlocked", "no seed to unlock"));
            }
            (present, unlocked)
        }
        Err(e) => {
            checks.push(Check::fail("seed", format!("cannot locate ~/.dig: {e}")));
            (false, false)
        }
    };

    // 3. Funds vs the publish cost (the per-capsule DIG amount + the XCH fee). Only
    //    scan when the seed is unlocked, so `doctor` never blocks on a passphrase prompt.
    let need_dig = dig::COMMIT_DIG;
    if seed_present && seed_unlocked {
        match scan_balances(ui) {
            Ok((have_dig, have_xch, fee)) => {
                let dig_ok = have_dig >= need_dig;
                let xch_ok = have_xch >= fee;
                let detail = format!(
                    "{} DIG / {} XCH (need {} DIG + ~{} XCH fee per publish)",
                    format_dig(have_dig),
                    format_xch(have_xch),
                    format_dig(need_dig),
                    format_xch(fee),
                );
                if dig_ok && xch_ok {
                    checks.push(Check::pass("funds", detail));
                } else if !dig_ok {
                    // Short on DIG: name where to get $DIG so the check is actionable.
                    checks.push(Check::fail(
                        "funds",
                        format!("{detail} — {}", crate::branding::get_dig_hint()),
                    ));
                } else {
                    checks.push(Check::fail("funds", detail));
                }
            }
            Err(e) => checks.push(Check::skip("funds", format!("could not scan wallet: {e}"))),
        }
    } else {
        checks.push(Check::skip(
            "funds",
            format!(
                "unlock the seed to check funds (need {} DIG + an XCH fee per publish)",
                format_dig(need_dig)
            ),
        ));
    }

    // 4. dighub login (gates `push` to the default DIGHUb remote).
    match crate::ops::dighub::load_session() {
        Some(s) if !s.is_expired() && !s.access_token.is_empty() => {
            let who = s.handle.clone().unwrap_or_else(|| "logged in".to_string());
            checks.push(Check::pass("dighub login", who));
        }
        Some(_) => checks.push(Check::fail(
            "dighub login",
            "session expired — run `digstore login`",
        )),
        None => checks.push(Check::fail(
            "dighub login",
            "not logged in — run `digstore login`",
        )),
    }

    // 5. Default remote reachable.
    let remote_url = crate::config::resolve_remote_url(ctx, "origin")
        .unwrap_or_else(|_| "https://rpc.dig.net".to_string());
    match remote_reachable(&remote_url) {
        Ok(true) => checks.push(Check::pass(
            "default remote",
            format!("{remote_url} reachable"),
        )),
        Ok(false) => checks.push(Check::fail(
            "default remote",
            format!("{remote_url} did not respond"),
        )),
        Err(e) => checks.push(Check::skip("default remote", format!("{remote_url}: {e}"))),
    }

    // 6. Content/output directory exists (from dig.toml/env or the default).
    let file = DigToml::read_with_env(&ctx.op_dir).unwrap_or_default();
    let content_rel = file.output_dir.unwrap_or_else(|| ".".to_string());
    let content_dir = if std::path::Path::new(&content_rel).is_absolute() {
        std::path::PathBuf::from(&content_rel)
    } else {
        ctx.op_dir.join(&content_rel)
    };
    if content_dir.is_dir() {
        checks.push(Check::pass(
            "content dir",
            content_dir.display().to_string(),
        ));
    } else {
        checks.push(Check::fail(
            "content dir",
            format!(
                "'{}' does not exist (build it, or set output-dir in dig.toml)",
                content_dir.display()
            ),
        ));
    }

    emit(ui, &checks)
}

/// Scan the wallet once (unlocked seed) and return `(dig, xch, fee)`. Uses the
/// shared anchor gate so the mock backend is honored in tests/CI.
fn scan_balances(ui: &Ui) -> Result<(u64, u64, u64), CliError> {
    let (_keys, mnemonic, anchor, _mocked, fee) = crate::ops::anchor_backend::prepare_anchor(ui)?;
    let w = block_on(anchor.scan(&mnemonic))??;
    let have_xch = block_on(anchor.balance(&w))??;
    let have_dig = block_on(anchor.dig_balance(&w))??;
    Ok((have_dig, have_xch, fee))
}

/// A lightweight reachability probe: GET the remote base and accept ANY HTTP
/// response (even 4xx) as "reachable" — we are checking the network/host, not
/// authorization. Honors the offline test override.
fn remote_reachable(url: &str) -> Result<bool, CliError> {
    if std::env::var_os("DIGSTORE_DOCTOR_REMOTE_OK").is_some() {
        return Ok(true);
    }
    if std::env::var_os("DIGSTORE_DOCTOR_REMOTE_DOWN").is_some() {
        return Ok(false);
    }
    // Probe the host root, not a store path (no store id needed to test reach).
    let base = base_url(url);
    block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| CliError::Network(e.to_string()))?;
        match client.get(&base).send().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    })?
}

/// The scheme+host (drop any `/stores/...` path) so we probe the node, not a store.
fn base_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or(rest);
        format!("https://{host}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split('/').next().unwrap_or(rest);
        format!("http://{host}")
    } else {
        url.to_string()
    }
}

/// Render the checks (human table or JSON) and return a non-zero error if any
/// HARD check failed (skips/soft notes do not fail doctor).
fn emit(ui: &Ui, checks: &[Check]) -> Result<(), CliError> {
    let any_fail = checks.iter().any(|c| c.status == Some(false));

    if ui.json() {
        let arr: Vec<_> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "check": c.name,
                    "status": match c.status {
                        Some(true) => "pass",
                        Some(false) => "fail",
                        None => "skip",
                    },
                    "detail": c.detail,
                })
            })
            .collect();
        ui.emit_json(&serde_json::json!({
            "ok": !any_fail,
            "checks": arr,
        }));
    } else {
        ui.line("Pre-publish checks:");
        for c in checks {
            let mark = match c.status {
                Some(true) => "✓",
                Some(false) => "✗",
                None => "•",
            };
            ui.line(format!("  {mark} {:<16} {}", c.name, c.detail));
        }
        if any_fail {
            ui.line("");
            ui.line("Some checks failed — fix them before `digstore deploy`.");
        } else {
            ui.line("");
            ui.success("Ready to publish.");
        }
    }

    if any_fail {
        // A failing preflight is an actionable, non-zero exit (invalid state to
        // publish from) without being a hard crash.
        Err(CliError::InvalidArgument(
            "one or more pre-publish checks failed".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_strips_store_path() {
        assert_eq!(
            base_url("https://rpc.dig.net/stores/abc"),
            "https://rpc.dig.net"
        );
        assert_eq!(base_url("https://rpc.dig.net"), "https://rpc.dig.net");
        assert_eq!(base_url("http://127.0.0.1:8443/x"), "http://127.0.0.1:8443");
    }

    #[test]
    fn emit_fails_when_any_check_fails() {
        let ui = Ui::resolve(
            crate::ui::ColorChoice::Never,
            true,
            true,
            true,
            false,
            false,
        );
        let checks = vec![Check::pass("a", "ok"), Check::fail("b", "bad")];
        assert!(emit(&ui, &checks).is_err());
    }

    #[test]
    fn emit_ok_when_all_pass_or_skip() {
        let ui = Ui::resolve(
            crate::ui::ColorChoice::Never,
            true,
            true,
            true,
            false,
            false,
        );
        let checks = vec![Check::pass("a", "ok"), Check::skip("b", "n/a")];
        assert!(emit(&ui, &checks).is_ok());
    }
}
