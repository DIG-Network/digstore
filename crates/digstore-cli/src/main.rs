//! The `digstore` binary — a thin shim over the shared entrypoint
//! [`digstore_cli::run`]. The `digs` alias binary (`src/bin/digs.rs`, issue #434)
//! shares this exact codepath, so the two binaries are identical modulo the
//! invoked program name (which clap derives from arg0).

fn main() {
    digstore_cli::run()
}
