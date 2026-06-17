use crate::cli::LogoutArgs;
use crate::error::CliError;
use crate::ops::dighub;

pub fn run(ui: &crate::ui::Ui, _args: LogoutArgs) -> Result<(), CliError> {
    // Idempotent: fine if there was no session.
    dighub::clear_session()?;
    if ui.json() {
        ui.emit_json(&serde_json::json!({ "logged_out": true }));
    } else {
        ui.success("Logged out");
    }
    Ok(())
}
