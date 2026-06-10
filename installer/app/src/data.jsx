/* Static data arrays — copy lifted verbatim from the prototype
   (design/installer/installer-app.jsx). Keep technically accurate to
   product/digstore-spec.txt (URN shape, AES-256-GCM, merkle, attestation). */
import { Ic } from "./icons.jsx";

export const STEPS = ["Welcome", "License", "Components", "Install", "Done"];

export const FEATURES = [
  {
    ic: Ic.git,
    h: "A Git-shaped workflow",
    p: "init, add, commit, log, diff, checkout, clone — the verbs you already know. Chunking, encryption and WASM compilation stay under the surface.",
  },
  {
    ic: Ic.lock,
    h: "Encrypted at rest, by URN",
    p: "Every URN is a key. Content is chunked, SHA-256 addressed, and sealed with an AES-256-GCM key derived from the URN itself.",
  },
  {
    ic: Ic.shield,
    h: "Provable & secretless",
    p: "Each store compiles to one portable .wasm that defends itself — merkle proofs, host attestation, and zero-knowledge proofs of execution. No embedded secret to extract.",
  },
];

// Component sizes/descriptions match the prototype. `id` values map 1:1 to the
// component identifiers the Rust install pipeline understands.
export const COMPONENTS = [
  { id: "cli", name: "DigStore CLI", desc: "The digstore command — init, add, commit, log, clone.", size: "18.4 MB", req: true },
  { id: "host", name: "Host Runtime", desc: "Sandboxed WASM host with attestation + session ABI.", size: "21.0 MB", on: true },
  { id: "completions", name: "Shell completions", desc: "bash · zsh · fish tab-completion for digstore.", size: "0.3 MB", on: true },
  { id: "path", name: "Add digstore to PATH", desc: "Symlink digstore into /usr/local/bin.", size: "—", on: true },
  { id: "example", name: "Example store", desc: "A sample urn:dig store to clone and explore.", size: "6.1 MB", on: false },
];

// Files surfaced in the progress header "writing <file>" while the real
// pipeline runs (the Rust side overrides these with the actual current file).
export const NOW_FILES = [
  "bin/digstore",
  "lib/dig_host.wasm",
  "lib/compiler.wasm",
  "share/completions/_digstore",
  "trusted/host-keys.toml",
  "examples/hello.wasm",
];
