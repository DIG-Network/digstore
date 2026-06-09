use std::io::Write;
use std::path::{Path, PathBuf};

use digstore_core::Bytes32;

use crate::error::Result;

/// The exact output filename: `{hex(store_id)}-{hex(roothash)}.dig` (§19.4).
/// The compiled store module is a WebAssembly binary, but it is distributed with
/// the `.dig` extension (it is a Digstore store, not a generic `.wasm`).
pub fn output_filename(store_id: &Bytes32, roothash: &Bytes32) -> String {
    format!(
        "{}-{}.dig",
        hex::encode(store_id.0),
        hex::encode(roothash.0)
    )
}

/// Write `bytes` atomically: write to `<final>.tmp` in the same directory, flush +
/// sync, then rename over the final path (§19.4).
pub fn atomic_write_module(
    dir: &Path,
    store_id: &Bytes32,
    roothash: &Bytes32,
    bytes: &[u8],
) -> Result<PathBuf> {
    let final_path = dir.join(output_filename(store_id, roothash));
    let tmp_path = dir.join(format!("{}.tmp", output_filename(store_id, roothash)));
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use digstore_core::Bytes32;

    #[test]
    fn output_filename_is_hex_store_dash_hex_root_dot_dig() {
        let sid = Bytes32([0xAB; 32]);
        let root = Bytes32([0x01; 32]);
        let name = output_filename(&sid, &root);
        assert_eq!(
            name,
            "abababababababababababababababababababababababababababababababab-\
0101010101010101010101010101010101010101010101010101010101010101.dig"
        );
    }

    #[test]
    fn atomic_write_creates_final_file_with_contents_and_no_temp_leftover() {
        let dir = std::env::temp_dir().join(format!("digc-aw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sid = Bytes32([1; 32]);
        let root = Bytes32([2; 32]);
        let bytes = vec![0xDEu8, 0xAD, 0xBE, 0xEF];
        let path = atomic_write_module(&dir, &sid, &root, &bytes).unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            output_filename(&sid, &root)
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file not renamed away");
        std::fs::remove_dir_all(&dir).ok();
    }
}
