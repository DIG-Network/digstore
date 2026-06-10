use digstore_core::{Bytes32, RevocationReason};

use crate::cli::RevokeArgs;
use crate::config;
use crate::context::CliContext;
use crate::error::CliError;
use crate::ops::remote_ops::{self, RevokeScope};

fn parse_reason(s: &str) -> Result<RevocationReason, CliError> {
    match s {
        "unspecified" => Ok(RevocationReason::Unspecified),
        "compromise" => Ok(RevocationReason::Compromise),
        "superseded" => Ok(RevocationReason::Superseded),
        "takedown" => Ok(RevocationReason::Takedown),
        other => Err(CliError::InvalidArgument(format!(
            "unknown --reason {other}: expected unspecified|compromise|superseded|takedown"
        ))),
    }
}

pub fn run(ctx: &CliContext, ui: &crate::ui::Ui, args: RevokeArgs) -> Result<(), CliError> {
    let reason = parse_reason(&args.reason)?;

    // Exactly one of --root / --all must be given (clap already rejects both).
    let root = match (&args.root, args.all) {
        (Some(hex), false) => Some(
            Bytes32::from_hex(hex)
                .map_err(|_| CliError::InvalidArgument(format!("bad --root hex: {hex}")))?,
        ),
        (None, true) => None,
        (None, false) => {
            return Err(CliError::InvalidArgument(
                "specify --root <hex> to revoke one generation, or --all to revoke the whole store"
                    .into(),
            ))
        }
        (Some(_), true) => unreachable!("clap conflicts_with rejects --root with --all"),
    };

    let base = config::resolve_remote_url(ctx, &args.remote)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Other(e.into()))?;
    let scope = rt.block_on(remote_ops::revoke_to(ctx, &base, root, reason))?;

    match scope {
        RevokeScope::Root(r) => {
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "revoked": "root",
                    "root": r.to_hex(),
                    "reason": args.reason,
                }));
            } else {
                ui.success(format!(
                    "revoked root {} ({}) on {}",
                    r.to_hex(),
                    args.reason,
                    args.remote
                ));
            }
        }
        RevokeScope::Store => {
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "revoked": "store",
                    "reason": args.reason,
                }));
            } else {
                ui.success(format!(
                    "revoked the entire store ({}) on {}",
                    args.reason, args.remote
                ));
            }
        }
    }
    Ok(())
}
