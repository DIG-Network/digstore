//! Command dispatch: clap `Command` -> `ops` -> `output`.

use crate::cli::{Cli, Command};
use crate::context::CliContext;
use crate::error::CliError;

pub mod add;
pub mod anchor;
pub mod balance;
pub mod cat;
pub mod checkout;
pub mod clone;
pub mod commit;
pub mod compile;
pub mod diff;
pub mod dir;
pub mod init;
pub mod keys;
pub mod lock;
pub mod log;
pub mod pull;
pub mod push;
pub mod remote;
pub mod revoke;
pub mod seed;
pub mod serve;
pub mod staged;
pub mod status;
pub mod stores;
pub mod unstage;
pub mod update;
pub mod urn;
pub mod use_store;

pub fn dispatch(cli: Cli) -> Result<(), CliError> {
    let ui = crate::ui::Ui::from_flags(
        cli.color,
        cli.json,
        cli.quiet,
        cli.verbose,
        cli.non_interactive,
        cli.yes,
    );
    let cwd = std::env::current_dir().map_err(|e| CliError::Other(e.into()))?;

    // `init` and `clone` CREATE a store, so they anchor to CWD/.dig (no walk-up,
    // like `git init`/`git clone`); `compile` is a self-contained headless build
    // into an ephemeral `.dig` (the caller passes --dig-dir); everything else
    // discovers an existing workspace by walking up.
    let workspace_dir = if matches!(
        cli.command,
        Command::Init(_) | Command::Clone(_) | Command::Compile(_)
    ) {
        CliContext::init_workspace(cli.dig_dir.clone())
    } else {
        CliContext::discover_workspace(cli.dig_dir.clone())
    };

    // init/clone create the workspace+store themselves; all other commands load
    // (and migrate) the workspace first.
    match cli.command {
        Command::Init(a) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return init::run(&ctx, &ui, a);
        }
        Command::Clone(a) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return clone::run(&ctx, &ui, a);
        }
        // `compile` builds an ephemeral single-store context at the (temp) workspace
        // dir, with op_dir == the --in content root, and never touches the chain.
        Command::Compile(a) => {
            let ctx = CliContext {
                dig_dir: workspace_dir.clone(),
                workspace_dir,
                op_dir: a.r#in.clone(),
                store_name: Some("default".to_string()),
                json: cli.json,
                verbose: cli.verbose,
            };
            return compile::run(&ctx, &ui, a);
        }
        Command::Stores(a) => {
            let ws = crate::workspace::Workspace::load_or_migrate(&workspace_dir)?;
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return stores::run(&ctx, &ui, &ws, a);
        }
        Command::Use(a) => {
            let mut ws = crate::workspace::Workspace::load_or_migrate(&workspace_dir)?;
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return use_store::run(&ctx, &ui, &mut ws, a);
        }
        // `update` is store-independent (it self-updates the binary), so it does
        // not load or migrate a workspace.
        Command::Update(a) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return update::run(&ctx, &ui, a);
        }
        Command::Seed(a) => return seed::run(&ui, a),
        Command::Lock(_) => return lock::run(&ui),
        // `balance` is wallet-only (it derives keys from the seed and queries the
        // anchor backend); it needs no store, like `seed`/`lock`.
        Command::Balance(_) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return balance::run(&ctx, &ui);
        }
        _ => {}
    }

    // Store-scoped commands: resolve the workspace, the store name, and op_dir.
    let ws = crate::workspace::Workspace::load_or_migrate(&workspace_dir)?;
    let name = ws.resolve_store_name(cli.store_name.as_deref())?;
    let content_root = ws.content_root(&name);
    let ctx = CliContext::for_store_with_op(
        workspace_dir,
        &name,
        content_root,
        cli.cwd.clone(),
        cwd,
        cli.json,
        cli.verbose,
    );

    match cli.command {
        Command::Add(a) => add::run(&ctx, &ui, a),
        Command::Commit(a) => commit::run(&ctx, &ui, a),
        Command::Status(a) => status::run(&ctx, &ui, a),
        Command::Log(a) => log::run(&ctx, &ui, a),
        Command::Diff(a) => diff::run(&ctx, &ui, a),
        Command::Checkout(a) => checkout::run(&ctx, &ui, a),
        Command::Cat(a) => cat::run(&ctx, &ui, a),
        Command::Keys(a) => keys::run(&ctx, &ui, a),
        Command::Dir(a) => dir::run(&ctx, &ui, a),
        Command::Unstage(a) => unstage::run(&ctx, &ui, a),
        Command::Staged(a) => staged::run(&ctx, &ui, a),
        Command::Urn(a) => urn::run(&ctx, &ui, a),
        Command::Remote(a) => remote::run(&ctx, &ui, a),
        Command::Push(a) => push::run(&ctx, &ui, a),
        Command::Pull(a) => pull::run(&ctx, &ui, a),
        Command::Revoke(a) => revoke::run(&ctx, &ui, a),
        Command::Serve(a) => serve::run(&ctx, &ui, a),
        Command::Anchor(a) => anchor::run(&ctx, &ui, a),
        Command::Init(_)
        | Command::Clone(_)
        | Command::Compile(_)
        | Command::Stores(_)
        | Command::Use(_)
        | Command::Update(_)
        | Command::Seed(_)
        | Command::Lock(_)
        | Command::Balance(_) => {
            unreachable!("handled above")
        }
    }
}
