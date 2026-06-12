// Runtime smoke test: drive the compiled wasm (nodejs target) against a fixture
// produced by the REAL host crypto (examples/gen_smoke_fixture.rs). Proves the
// emitted bundle's three primitives work end-to-end in a JS runtime:
//   (1) URN reconstruction + retrieval key
//   (2) inclusion-proof verification against the trusted (chain) root
//   (3) AES-256-GCM-SIV decryption to the original plaintext
// Exit 0 = all assertions pass.
import { readFileSync } from "node:fs";
import * as dig from "./pkg-node/dig_client.js";

const fx = JSON.parse(readFileSync(new URL("./smoke_fixture.json", import.meta.url)));
const ct = Uint8Array.from(Buffer.from(fx.ciphertext_b64, "base64"));
let fails = 0;
const check = (name, cond) => {
  console.log(`${cond ? "ok  " : "FAIL"} ${name}`);
  if (!cond) fails++;
};

// (1) URN reconstruction
check("reconstructUrn", dig.reconstructUrn(fx.store_id, fx.resource_key) === fx.expected_urn);
check("retrievalKey", dig.retrievalKey(fx.store_id, fx.resource_key) === fx.expected_retrieval_key);
check("reconstructUrn empty -> index.html",
  dig.reconstructUrn(fx.store_id, "") === fx.expected_urn);

// (3) key derivation is deterministic + 64 hex
const key = dig.deriveKey(fx.store_id, fx.resource_key, null);
check("deriveKey is 64-hex", /^[0-9a-f]{64}$/.test(key));
check("deriveKey deterministic", dig.deriveKey(fx.store_id, fx.resource_key, null) === key);

// (2) inclusion proof verifies against the real root, rejects the wrong root
check("verifyInclusion true vs real root", dig.verifyInclusion(ct, fx.proof_b64, fx.root) === true);
check("verifyInclusion false vs wrong root (decoy path)",
  dig.verifyInclusion(ct, fx.proof_b64, fx.wrong_root) === false);

// tampered ciphertext -> false
const tampered = Uint8Array.from(ct); tampered[0] ^= 0xff;
check("verifyInclusion false on tampered bytes",
  dig.verifyInclusion(tampered, fx.proof_b64, fx.root) === false);

// (3) full pipeline decrypt to text (gate-then-decrypt)
const text = dig.decryptResourceToText(fx.store_id, fx.resource_key, ct, fx.proof_b64, fx.root, null, null);
check("decryptResourceToText matches plaintext", text === fx.expected_plaintext);

// decrypt must REFUSE when the proof does not chain to the trusted root.
let refused = false;
try { dig.decryptResource(fx.store_id, fx.resource_key, ct, fx.proof_b64, fx.wrong_root, null, null); }
catch { refused = true; }
check("decryptResource throws on wrong trusted root", refused);

// globalThis.digClient installed by the start hook.
check("globalThis.digClient installed", typeof globalThis.digClient?.verifyInclusion === "function");
check("digClient.decryptResourceToText round-trips",
  globalThis.digClient.decryptResourceToText(fx.store_id, fx.resource_key, ct, fx.proof_b64, fx.root, null, null) === fx.expected_plaintext);

console.log(fails === 0 ? "\nSMOKE PASS" : `\nSMOKE FAIL (${fails})`);
process.exit(fails === 0 ? 0 : 1);
