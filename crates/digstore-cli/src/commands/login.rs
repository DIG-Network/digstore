use crate::cli::LoginArgs;
use crate::error::CliError;
use crate::ops::dighub;

pub fn run(ui: &crate::ui::Ui, _args: LoginArgs) -> Result<(), CliError> {
    // JSON / non-interactive: never block-poll a non-TTY forever. Pair, print the
    // pairing info, then poll quietly (honoring expires_in) so automation can still
    // complete a login it has approved out-of-band — but it can never hang.
    if ui.json() || !ui.can_prompt() {
        let base = dighub::api_base();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| CliError::Other(e.into()))?;
        let pairing = rt.block_on(dighub::pair(&base))?;
        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "user_code": pairing.user_code,
                "verification_uri": pairing.verification_uri,
                "interval": pairing.interval,
                "expires_in": pairing.expires_in,
            }));
        }
        // Quietly poll (no spinner; the spinner is auto-hidden in json/non-tty)
        // until approval or expiry, so a non-TTY never hangs past expires_in.
        let session = rt.block_on(dighub::poll_until_approved_quiet(&base, &pairing))?;
        dighub::save_session(&session)?;
        if ui.json() {
            ui.emit_json(&serde_json::json!({
                "logged_in": true,
                "handle": session.handle,
            }));
        }
        return Ok(());
    }

    let session = dighub::login_interactive(ui)?;
    match session.handle {
        Some(h) => ui.success(format!("Logged in as @{h}")),
        None => ui.success("Logged in."),
    }
    Ok(())
}
