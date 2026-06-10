use crate::context::CliContext;
use crate::error::CliError;
use crate::ui::Ui;
use crate::workspace::Workspace;

pub fn run(
    _ctx: &CliContext,
    _ui: &Ui,
    _ws: &Workspace,
    _args: crate::cli::StoresArgs,
) -> Result<(), CliError> {
    unimplemented!()
}
