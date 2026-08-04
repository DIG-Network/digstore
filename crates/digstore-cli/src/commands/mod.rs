//! Command dispatch: clap `Command` -> `ops` -> `output`.

use crate::cli::{Cli, Command};
use crate::context::CliContext;
use crate::error::CliError;

pub mod add;
pub mod anchor;
pub mod authorize_origin;
pub mod balance;
pub mod cat;
pub mod checkout;
pub mod clone;
pub mod collection;
pub mod commit;
pub mod compile;
pub mod completion;
pub mod config;
pub mod deploy;
pub mod deploy_key;
pub mod dev;
pub mod did;
pub mod diff;
pub mod dir;
pub mod doctor;
pub mod init;
pub mod keys;
pub mod link;
pub mod lock;
pub mod log;
pub mod login;
pub mod logout;
pub mod manifest;
pub mod new;
pub mod nft;
pub mod offer;
pub mod pull;
pub mod push;
pub mod remote;
pub mod revoke;
pub mod seed;
pub mod serve;
pub mod setup;
pub mod staged;
pub mod status;
pub mod store_status;
pub mod stores;
pub mod unstage;
pub mod update;
pub mod urn;
pub mod use_store;
pub mod whoami;

pub fn dispatch(cli: Cli) -> Result<(), CliError> {
    let ui =
        crate::ui::Ui::from_flags(cli.color, cli.json, cli.quiet, cli.non_interactive, cli.yes);
    let cwd = std::env::current_dir().map_err(|e| CliError::Other(e.into()))?;

    // `init` and `clone` CREATE a store, so they anchor to CWD/.dig (no walk-up,
    // like `git init`/`git clone`); `compile` is a self-contained headless build
    // into an ephemeral `.dig` (the caller passes --dig-dir); everything else
    // discovers an existing workspace by walking up.
    // `deploy` also CREATES/adopts the store from a fresh checkout (like
    // init/clone), so it anchors to CWD/.dig with no walk-up.
    // `new`/`dev`/`doctor` do not require an existing `.dig` workspace: `new`
    // only writes template files; `dev` builds into an ephemeral scratch `.dig`;
    // `doctor` reads dig.toml + config from CWD. Like init/deploy they anchor to
    // CWD/.dig with no walk-up (they never load a real workspace).
    let workspace_dir = if matches!(
        cli.command,
        Command::Init(_)
            | Command::Clone(_)
            | Command::Compile(_)
            | Command::Deploy(_)
            | Command::New(_)
            | Command::Dev(_)
            | Command::Doctor(_)
            | Command::Link(_)
            | Command::Nft(_)
    ) {
        CliContext::init_workspace(cli.dig_dir.clone())
    } else {
        CliContext::discover_workspace(cli.dig_dir.clone())
    };

    // init/clone create the workspace+store themselves; all other commands load
    // (and migrate) the workspace first.
    match cli.command {
        // `new` is workspace-independent: it scaffolds template files into a target
        // dir. No chain, no spend, no `.dig`.
        Command::New(a) => return new::run(&ui, a),
        // `dev` and `doctor` operate on CWD (where `dig.toml` + content live),
        // never walk up, and own a CWD-anchored op_dir context like `deploy`.
        Command::Dev(a) => {
            let ctx = CliContext {
                dig_dir: workspace_dir.clone(),
                workspace_dir,
                op_dir: cwd,
                store_name: Some("default".to_string()),
                json: cli.json,
                verbose: cli.verbose,
            };
            return dev::run(&ctx, &ui, a);
        }
        Command::Doctor(a) => {
            let ctx = CliContext {
                dig_dir: workspace_dir.clone(),
                workspace_dir,
                op_dir: cwd,
                store_name: Some("default".to_string()),
                json: cli.json,
                verbose: cli.verbose,
            };
            return doctor::run(&ctx, &ui, a, cli.node.as_deref());
        }
        // `link` writes a committable `dig.toml` into CWD (where the developer's
        // source lives); like `doctor` it operates on the op_dir and needs no
        // existing workspace/store.
        Command::Link(a) => {
            let ctx = CliContext {
                dig_dir: workspace_dir.clone(),
                workspace_dir,
                op_dir: cwd,
                store_name: Some("default".to_string()),
                json: cli.json,
                verbose: cli.verbose,
            };
            return link::run(&ctx, &ui, a);
        }
        Command::Init(a) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return init::run(&ctx, &ui, a);
        }
        Command::Clone(a) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return clone::run(&ctx, &ui, a, cli.node.as_deref());
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
        // `deploy` advances an EXISTING store from a fresh checkout (CI). It adopts
        // the store into `<workspace>/stores/default` (reconstructing its local
        // state) and runs add+commit+push, so it owns its own store-scoped context
        // with op_dir == CWD (where `dig.toml` and the build output live).
        Command::Deploy(a) => {
            let ctx = CliContext {
                dig_dir: workspace_dir.join("stores").join("default"),
                workspace_dir,
                op_dir: cwd,
                store_name: Some("default".to_string()),
                json: cli.json,
                verbose: cli.verbose,
            };
            return deploy::run(&ctx, &ui, a, cli.node.as_deref());
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
        // `config` is workspace-independent: it reads/writes the global dig
        // config dir (identity_dir), the same home `seed`/`login` use — not a
        // per-store setting, so no CliContext with a resolved store is needed.
        // Passed a workspace-only context purely to satisfy the `run` signature
        // shape shared with other commands; `node.url` itself ignores it.
        Command::Config(a) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return config::run(&ctx, &ui, a);
        }
        // `setup`/`auth` guides seed + fund check + optional login; like `seed`/
        // `login` it is workspace-independent (it touches the identity dir, not a
        // store). `completion` just prints a static script.
        Command::Setup(a) => return setup::run(&ui, a),
        Command::Completion(a) => return completion::run(&ui, a.shell),
        // Wave-B asset commands. `nft` needs a CWD-anchored context (its `mint` subcommand builds an
        // ephemeral media capsule under `<workspace>/.dig`, like `compile`); `did`/`offer`/
        // `collection` are wallet-only (they derive keys + push, no store), like `balance`.
        Command::Nft(a) => {
            let ctx = CliContext {
                dig_dir: workspace_dir.join("stores").join("default"),
                workspace_dir,
                op_dir: cwd,
                store_name: Some("default".to_string()),
                json: cli.json,
                verbose: cli.verbose,
            };
            return nft::run(&ctx, &ui, a);
        }
        Command::Did(a) => return did::run(&ui, a),
        Command::Offer(a) => return offer::run(&ui, a),
        Command::Collection(a) => return collection::run(&ui, a),
        // dighub account commands: workspace-independent (the session lives next to
        // the identity key, not in any store).
        Command::Login(a) => return login::run(&ui, a),
        Command::Whoami(a) => return whoami::run(&ui, a),
        Command::Logout(a) => return logout::run(&ui, a),
        // `balance` is wallet-only (it derives keys from the seed and queries the
        // anchor backend); it needs no store, like `seed`/`lock`.
        Command::Balance(_) => {
            let ctx = CliContext::workspace_only(workspace_dir, cli.json, cli.verbose);
            return balance::run(&ctx, &ui);
        }
        // `store-status` reads a store's on-chain status by store id alone — it needs no local
        // store/workspace (like `did`/`offer`), and reads raw Chia chain state via coinset (NOT
        // the §5.3 dig-node content ladder). See `store_status` for the endpoint resolution.
        Command::StoreStatus(a) => return store_status::run(&ui, a),
        _ => {}
    }

    // `cat` needs a store-resolution rule DIFFERENT from every other store-scoped command: a
    // full `urn:dig:…` argument is self-contained (§5.3 — it carries the store id and, usually,
    // a pinned root) and MUST remain readable via the node ladder even when NO local store is
    // registered at all (#227). The bare 64-hex retrieval-key form still legitimately requires
    // the active local store, so it is unaffected — `cat::run` itself decides per-argument (see
    // its doc comment); it also falls through to the network whenever a resolved local store
    // does not match the URN's store id.
    if let Command::Cat(a) = cli.command {
        let ws = crate::workspace::Workspace::load_or_migrate(&workspace_dir)?;
        let ctx = match ws.resolve_store_name(cli.store_name.as_deref()) {
            Ok(name) => {
                let content_root = ws.content_root(&name);
                CliContext::for_store_with_op(
                    workspace_dir,
                    &name,
                    content_root,
                    cli.cwd.clone(),
                    cwd,
                    cli.json,
                    cli.verbose,
                )
            }
            // No local store at all is fine for a full URN (network path) — but a genuine
            // error for the bare retrieval-key form, which `cat::run` still enforces itself
            // via `ctx.load_config()` inside `cat_by_retrieval_key`.
            Err(CliError::NoStore(_)) => {
                CliContext::workspace_only(workspace_dir, cli.json, cli.verbose)
            }
            Err(e) => return Err(e),
        };
        return cat::run(&ctx, &ui, a, cli.node.as_deref());
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
        Command::Commit(a) => commit::run(&ctx, &ui, a, cli.node.as_deref()),
        Command::Status(a) => status::run(&ctx, &ui, a),
        Command::Log(a) => log::run(&ctx, &ui, a),
        Command::Diff(a) => diff::run(&ctx, &ui, a),
        Command::Checkout(a) => checkout::run(&ctx, &ui, a),
        Command::Keys(a) => keys::run(&ctx, &ui, a),
        Command::Manifest(a) => manifest::run(&ctx, &ui, a),
        Command::Dir(a) => dir::run(&ctx, &ui, a),
        Command::Unstage(a) => unstage::run(&ctx, &ui, a),
        Command::Staged(a) => staged::run(&ctx, &ui, a),
        Command::Urn(a) => urn::run(&ctx, &ui, a),
        Command::Remote(a) => remote::run(&ctx, &ui, a),
        Command::Push(a) => push::run(&ctx, &ui, a, cli.node.as_deref()),
        Command::Pull(a) => pull::run(&ctx, &ui, a, cli.node.as_deref()),
        Command::Revoke(a) => revoke::run(&ctx, &ui, a, cli.node.as_deref()),
        Command::Serve(a) => serve::run(&ctx, &ui, a),
        Command::Anchor(a) => anchor::run(&ctx, &ui, a),
        Command::DeployKey(a) => deploy_key::run(&ctx, &ui, a),
        Command::AuthorizeOriginAsWriter(a) => authorize_origin::run(&ctx, &ui, a),
        Command::New(_)
        | Command::Dev(_)
        | Command::Doctor(_)
        | Command::Link(_)
        | Command::Setup(_)
        | Command::Completion(_)
        | Command::Init(_)
        | Command::Clone(_)
        | Command::Compile(_)
        | Command::Deploy(_)
        | Command::Stores(_)
        | Command::Use(_)
        | Command::Update(_)
        | Command::Seed(_)
        | Command::Lock(_)
        | Command::Config(_)
        | Command::Balance(_)
        | Command::Login(_)
        | Command::Whoami(_)
        | Command::Logout(_)
        | Command::Nft(_)
        | Command::Did(_)
        | Command::Offer(_)
        | Command::Collection(_)
        | Command::StoreStatus(_)
        | Command::Cat(_) => {
            unreachable!("handled above")
        }
    }
}
