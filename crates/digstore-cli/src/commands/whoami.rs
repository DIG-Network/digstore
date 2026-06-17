use crate::cli::WhoamiArgs;
use crate::error::CliError;
use crate::ops::dighub;

pub fn run(ui: &crate::ui::Ui, _args: WhoamiArgs) -> Result<(), CliError> {
    match dighub::load_session() {
        Some(s) => {
            if ui.json() {
                ui.emit_json(&serde_json::json!({
                    "logged_in": true,
                    "handle": s.handle,
                    "has_token": !s.access_token.is_empty(),
                    "api_base": s.api_base,
                    "expired": s.is_expired(),
                }));
            } else {
                match &s.handle {
                    Some(h) => ui.line(format!("@{h}")),
                    None => ui.line("logged in (no handle set)"),
                }
                if s.access_token.is_empty() {
                    ui.line("(no token present)");
                } else if s.is_expired() {
                    ui.line("token present (expired — run `digstore login`)");
                } else {
                    ui.line("token present");
                }
            }
            Ok(())
        }
        None => {
            if ui.json() {
                ui.emit_json(&serde_json::json!({ "logged_in": false }));
            }
            // Non-zero exit. In human mode `ui.error` renders this as
            // `error: not logged in — run `digstore login``; in JSON mode the
            // body above is the machine-readable result and the error just sets
            // the exit code.
            Err(CliError::Unauthorized(
                "not logged in — run `digstore login`".into(),
            ))
        }
    }
}
