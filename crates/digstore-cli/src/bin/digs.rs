//! `digs` — a FIRST-CLASS alias binary for the `digstore` CLI (issue #434).
//!
//! `digs <args>` behaves IDENTICALLY to `digstore <args>`: same subcommands, flags,
//! `--json`, and help. It is a real installed binary (not a shell alias) that shares
//! the SINGLE entrypoint [`digstore_cli::run`] with `digstore` — there is no
//! duplicated logic. clap derives the displayed program name from arg0, so
//! `digs --help`/`--version`/`completion` all read `digs`.

fn main() {
    digstore_cli::run()
}
