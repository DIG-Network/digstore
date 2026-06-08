use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest_dir.join("fixtures/digstore_guest_template.wat");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("digstore_guest_template.wasm");

    let wat = std::fs::read_to_string(&src).expect("read template wat");
    let wasm = wat::parse_str(&wat).expect("assemble template wat");
    std::fs::write(&dest, wasm).expect("write template wasm");

    println!("cargo:rerun-if-changed=fixtures/digstore_guest_template.wat");
}
