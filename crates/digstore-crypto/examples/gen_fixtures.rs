//! Regenerates the committed fixture files under `tests/fixtures/`.
//! Run with: cargo run -p digstore-crypto --example gen_fixtures

use std::path::Path;

fn main() -> std::io::Result<()> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    digstore_crypto::write_kdf_fixtures(&base.join("kdf_kat.json"))?;
    println!("wrote {}", base.join("kdf_kat.json").display());
    Ok(())
}
