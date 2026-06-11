use crate::error::CliError;
use crate::ui::Ui;
use digstore_chain::{config, unlock};

pub fn run(ui: &Ui) -> Result<(), CliError> {
    let home = config::dig_home().map_err(CliError::from)?;
    unlock::clear_session(&config::session_path(&home)).map_err(CliError::from)?;
    ui.success("seed locked");
    Ok(())
}
