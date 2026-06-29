// verify-pkg.mjs — gate the assembled `@dignetwork/dig-client` package (roadmap #16).
//
// Asserts the PUBLISHABLE pkg/ is correct end-to-end before it can ship:
//   1. package.json: scoped name, public access, conditional exports, files set.
//   2. The shipped wasm digest matches package.json `digIntegrity` + integrity.json.
//   3. The Node entry actually LOADS the wasm and runs the read path against a
//      fixture produced by the REAL host crypto (gen_smoke_fixture) — the same
//      smoke contract as smoke.mjs, but exercising the assembled package layout.
//   4. The .d.ts exports the full typed surface (every wasm_bindgen export).
//   5. The runtime `version()` equals the package version (SRI/compat anchor).
//
// Exit 0 = the package is publishable. Run via `npm run test:pkg` (which first
// builds pkg/ if needed) or directly after `npm run build:pkg`.

import { readFileSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const pkgDir = resolve(root, "pkg");

let fails = 0;
const check = (name, cond, detail = "") => {
  console.log(`${cond ? "ok  " : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
  if (!cond) fails++;
};

if (!existsSync(pkgDir)) {
  console.error("verify-pkg: pkg/ not assembled; run `npm run build:pkg` first");
  process.exit(1);
}

// --- (1) package.json shape -------------------------------------------------
const pkg = JSON.parse(readFileSync(resolve(pkgDir, "package.json"), "utf8"));
check("name is scoped", pkg.name === "@dignetwork/dig-client", pkg.name);
check("publishConfig.access is public", pkg.publishConfig?.access === "public");
check("type is module", pkg.type === "module");
check("version is semver", /^\d+\.\d+\.\d+/.test(pkg.version ?? ""), pkg.version);
check(
  "exports['.'] has node condition (require -> .cjs)",
  pkg.exports?.["."]?.node?.require === "./node/dig_client.cjs"
);
check("exports['.'] has browser condition", typeof pkg.exports?.["."]?.browser === "string");
check("exports['.'] has default condition", typeof pkg.exports?.["."]?.default === "string");
check("exports['.'] has types", typeof pkg.exports?.["."]?.types === "string");
check("exports exposes ./web", typeof pkg.exports?.["./web"] !== "undefined");
check("exports exposes ./node", typeof pkg.exports?.["./node"] !== "undefined");
check("exports exposes raw wasm", typeof pkg.exports?.["./dig_client_bg.wasm"] !== "undefined");

// --- (2) integrity: shipped wasm digest matches the manifest + integrity.json
const wasm = readFileSync(resolve(pkgDir, "dig_client_bg.wasm"));
const sha256 = createHash("sha256").update(wasm).digest("hex");
const sri = "sha384-" + createHash("sha384").update(wasm).digest("base64");
const integrity = JSON.parse(readFileSync(resolve(pkgDir, "integrity.json"), "utf8"));
check("integrity.json sha256 matches shipped wasm", integrity.sha256 === sha256, sha256);
check("integrity.json sri matches shipped wasm", integrity.sri === sri);
check("package.json digIntegrity.sha256 matches", pkg.digIntegrity?.sha256 === sha256);
check("integrity.json version matches package", integrity.version === pkg.version);

// --- (4) typed surface ------------------------------------------------------
const dts = readFileSync(resolve(pkgDir, "dig_client.d.ts"), "utf8");
for (const fn of [
  "reconstructUrn",
  "reconstructUrnWithRoot",
  "retrievalKey",
  "deriveKey",
  "decryptChunk",
  "encryptResource",
  "decryptResource",
  "decryptResourceToText",
  "verifyInclusion",
  "version",
]) {
  check(`.d.ts exports ${fn}`, new RegExp(`export function ${fn}\\b`).test(dts));
}
check(".d.ts exports default init", /export default function/.test(dts));

// --- (3)+(5) runtime: load the Node entry and run the read path -------------
const fixturePath = resolve(root, "smoke_fixture.json");
if (!existsSync(fixturePath)) {
  console.error(
    "verify-pkg: smoke_fixture.json missing; run `cargo run --example gen_smoke_fixture > smoke_fixture.json`"
  );
  process.exit(1);
}
const fx = JSON.parse(readFileSync(fixturePath, "utf8"));
const ct = Uint8Array.from(Buffer.from(fx.ciphertext_b64, "base64"));

// The Node entry is CommonJS (.cjs); load it via createRequire so we exercise the
// exact `require("@dignetwork/dig-client")` path a Node consumer uses.
const { createRequire } = await import("node:module");
const require = createRequire(import.meta.url);
const nodeEntry = resolve(pkgDir, "node", "dig_client.cjs");
const dig = require(nodeEntry);

check("node entry: version() equals package version", dig.version() === pkg.version, dig.version());
check("node entry: reconstructUrn", dig.reconstructUrn(fx.store_id, fx.resource_key) === fx.expected_urn);
check(
  "node entry: retrievalKey",
  dig.retrievalKey(fx.store_id, fx.resource_key) === fx.expected_retrieval_key
);
check(
  "node entry: verifyInclusion true vs real root",
  dig.verifyInclusion(ct, fx.proof_b64, fx.root) === true
);
check(
  "node entry: verifyInclusion false vs wrong root",
  dig.verifyInclusion(ct, fx.proof_b64, fx.wrong_root) === false
);
check(
  "node entry: decryptResourceToText round-trips",
  dig.decryptResourceToText(fx.store_id, fx.resource_key, ct, fx.proof_b64, fx.root, null, null) ===
    fx.expected_plaintext
);

console.log(fails === 0 ? "\nPKG VERIFY PASS" : `\nPKG VERIFY FAIL (${fails})`);
process.exit(fails === 0 ? 0 : 1);
