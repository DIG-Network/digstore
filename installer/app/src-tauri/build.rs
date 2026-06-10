//! Build script for the DigStore installer.
//!
//! In addition to the normal `tauri_build::build()`, this **embeds the prebuilt
//! `digstore` binary directly into the installer executable** so the shipped
//! artifact is a single self-contained file — no sidecar `resources/bin/` folder
//! at runtime. The release pipeline stages the binary into
//! `resources/bin/digstore[.exe]` (via `scripts/stage-binary.mjs`) before this
//! runs; we copy it into `OUT_DIR`, compute its SHA-256, and (when the host can
//! execute it) capture its `--version`.
//!
//! If no binary is staged (e.g. a plain `cargo check`/dev build), we simply do
//! not emit the `embed_digstore` cfg and the runtime falls back to the Tauri
//! resource directory. The crate therefore always compiles.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Let rustc know `embed_digstore` is an expected cfg (no warning when unset).
    println!("cargo:rustc-check-cfg=cfg(embed_digstore)");


    let bin_name = if cfg!(windows) {
        "digstore.exe"
    } else {
        "digstore"
    };

    // Source precedence: explicit DIGSTORE_BIN env, else the staged resource.
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let staged = manifest.join("resources").join("bin").join(bin_name);
    let src = match env::var_os("DIGSTORE_BIN") {
        Some(p) => PathBuf::from(p),
        None => staged.clone(),
    };

    println!("cargo:rerun-if-env-changed=DIGSTORE_BIN");
    println!("cargo:rerun-if-changed={}", staged.display());

    if src.is_file() {
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let bytes = fs::read(&src).expect("read staged digstore binary");

        // Embed the bytes.
        let embedded = out_dir.join("digstore.bin");
        fs::write(&embedded, &bytes).expect("write embedded digstore.bin");

        // Embed a SHA-256 of exactly what we embedded (the install-time gate
        // recomputes this over the written bytes and compares).
        let digest = sha256_hex(&bytes);
        fs::write(out_dir.join("digstore.sha256"), &digest).expect("write embedded sha");

        // Best-effort: capture the bundled CLI version so the UI badge is exact.
        // Only attempt to execute when host==target (no cross-compile), and never
        // fail the build if it doesn't run.
        let host = env::var("HOST").unwrap_or_default();
        let target = env::var("TARGET").unwrap_or_default();
        let version = if host == target {
            Command::new(&src)
                .arg("--version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .split_whitespace()
                        .nth(1)
                        .map(|s| s.to_string())
                })
        } else {
            None
        };
        if let Some(v) = version {
            println!("cargo:rustc-env=DIGSTORE_BUNDLED_VERSION={v}");
        }

        println!("cargo:rustc-cfg=embed_digstore");
    }

    // Build with an explicit asInvoker manifest. Tauri embeds a manifest by
    // default but does NOT set a requestedExecutionLevel, so Windows Installer
    // Detection heuristically auto-elevates this exe (it sees "installer" in the
    // file/version info) and forces a UAC prompt. This install is per-user
    // (%LOCALAPPDATA% + HKCU only) and needs no elevation, so pin asInvoker.
    let manifest = r#"<?xml version="1.0" encoding="utf-8"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{e2011457-1546-43c5-a5fe-008deee3d3f0}" />
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}" />
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}" />
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}" />
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" />
    </application>
  </compatibility>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>"#;
    let attributes = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new().app_manifest(manifest),
    );
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

/// Minimal SHA-256 (avoid a build-dep; build scripts compile separately from the
/// runtime crate, so we don't reuse its `sha2`).
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}
