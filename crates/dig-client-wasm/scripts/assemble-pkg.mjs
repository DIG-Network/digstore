// assemble-pkg.mjs — assemble the publishable `@dignetwork/dig-client` npm package.
//
// Roadmap #16: publish the read-crypto WASM (dig_client) to npm so consumers
// (hub.dig.net, dig-embed.js, dig-companion, dig-sdk) can `npm i` it instead of
// vendoring the artifact with a hand-copied SHA. This is the digstore analogue of
// chip35's `scripts/patch-pkg.mjs`, extended to ship BOTH wasm-bindgen targets in
// ONE package behind conditional exports.
//
// Inputs (produced by `wasm-pack` — run `npm run build:web` + `build:node` first):
//   pkg-web/  — `--target web`    : async `init()` default export + named exports.
//   pkg-node/ — `--target nodejs` : CommonJS-style, auto-initializes synchronously.
//
// The wasm BINARY is byte-identical across targets (only the JS glue differs), so
// we ship ONE `dig_client_bg.wasm` shared by both entries and pin ONE SHA-256.
//
// Output: pkg/ — a single publishable package:
//   pkg/package.json        scoped name, exports map, integrity, publishConfig
//   pkg/dig_client.d.ts     the typed surface (identical for both targets)
//   pkg/web/                the browser/bundler ES-module entry + glue
//   pkg/node/               the Node entry + glue
//   pkg/dig_client_bg.wasm  the shared read-crypto binary (the SRI anchor)
//   pkg/README.md           install + integrity docs
//   pkg/integrity.json      machine-readable { version, sha256, sri } (agent-friendly)
//
// Conditional exports: `node` resolves to ./node, `browser`/`import`/`default`
// resolve to ./web. Both web glue and node glue load `../dig_client_bg.wasm`,
// which we patch their relative path to point at after moving files.
//
// Idempotent: safe to re-run; it rebuilds pkg/ from the two target dirs each time.

import {
  readFileSync,
  writeFileSync,
  mkdirSync,
  rmSync,
  copyFileSync,
  existsSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const SCOPED_NAME = "@dignetwork/dig-client";
const REPO_URL = "https://github.com/DIG-Network/digstore";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const webDir = resolve(root, "pkg-web");
const nodeDir = resolve(root, "pkg-node");
const outDir = resolve(root, "pkg");

const verifyOnly = process.argv.includes("--verify-only");

function die(msg) {
  console.error(`assemble-pkg: ${msg}`);
  process.exit(1);
}

for (const d of [webDir, nodeDir]) {
  if (!existsSync(d)) {
    if (verifyOnly) process.exit(2); // signal "not built yet" for test:pkg
    die(`missing ${d}; run \`npm run build:web\` + \`npm run build:node\` first`);
  }
}

// 1) Both targets must emit the SAME wasm binary. If they ever diverge, refuse:
//    a single shared binary (one SRI anchor) is the whole point.
const webWasm = readFileSync(resolve(webDir, "dig_client_bg.wasm"));
const nodeWasm = readFileSync(resolve(nodeDir, "dig_client_bg.wasm"));
const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
if (sha256(webWasm) !== sha256(nodeWasm)) {
  die(
    `web and node wasm differ (${sha256(webWasm)} vs ${sha256(nodeWasm)}); ` +
      `expected byte-identical binaries`
  );
}
const wasmSha256 = sha256(webWasm);
const wasmSri = "sha384-" + createHash("sha384").update(webWasm).digest("base64");

// 2) Version comes from the Cargo crate (the single source of truth). The
//    `version()` wasm export returns the same string at runtime.
const cargoToml = readFileSync(resolve(root, "Cargo.toml"), "utf8");
const versionMatch = cargoToml.match(/^\s*version\s*=\s*"([^"]+)"/m);
if (!versionMatch) die("could not read version from Cargo.toml");
const version = versionMatch[1];

if (verifyOnly) {
  console.log(`assemble-pkg: would build ${SCOPED_NAME}@${version} sha256=${wasmSha256}`);
  process.exit(0);
}

// 3) (Re)create pkg/ from the two target dirs.
rmSync(outDir, { recursive: true, force: true });
mkdirSync(resolve(outDir, "web"), { recursive: true });
mkdirSync(resolve(outDir, "node"), { recursive: true });

// Shared wasm binary at the package root (one copy, one SRI anchor).
copyFileSync(resolve(webDir, "dig_client_bg.wasm"), resolve(outDir, "dig_client_bg.wasm"));

// Web glue → pkg/web/ ; patch its wasm path to the shared root copy.
const webGlue = readFileSync(resolve(webDir, "dig_client.js"), "utf8").replaceAll(
  "dig_client_bg.wasm",
  "../dig_client_bg.wasm"
);
writeFileSync(resolve(outDir, "web", "dig_client.js"), webGlue);

// Node glue → pkg/node/dig_client.cjs. The `--target nodejs` glue is CommonJS
// (`exports.*`, `require('fs')`), but the package is `"type": "module"`, which
// would make a `.js` file load as ESM and crash on `exports`. Ship it as `.cjs`
// so Node always treats it as CommonJS regardless of the package type. Patch its
// `${__dirname}/dig_client_bg.wasm` to the shared root copy one level up.
const nodeGlue = readFileSync(resolve(nodeDir, "dig_client.js"), "utf8").replaceAll(
  "dig_client_bg.wasm",
  "../dig_client_bg.wasm"
);
writeFileSync(resolve(outDir, "node", "dig_client.cjs"), nodeGlue);

// 4) Typed surface: identical for both targets. Ship one canonical .d.ts at the
//    package root AND beside each entry (so `types` resolves under any condition).
const dts = readFileSync(resolve(webDir, "dig_client.d.ts"), "utf8");
writeFileSync(resolve(outDir, "dig_client.d.ts"), dts);
writeFileSync(resolve(outDir, "web", "dig_client.d.ts"), dts);
// The CJS node entry needs a `.d.cts` alongside it so `types` resolves under the
// `node`/`require` condition. The typed surface is identical to the ESM .d.ts.
writeFileSync(resolve(outDir, "node", "dig_client.d.cts"), dts);

// 5) Machine-readable integrity (agent-friendly): pin the wasm digest + SRI so a
//    consumer can verify the exact binary regardless of the npm semver.
const integrity = {
  package: SCOPED_NAME,
  version,
  wasm: "dig_client_bg.wasm",
  sha256: wasmSha256,
  sri: wasmSri,
};
writeFileSync(resolve(outDir, "integrity.json"), JSON.stringify(integrity, null, 2) + "\n");

// 6) The publishable package.json. Conditional exports route Node vs browser to
//    the matching wasm-bindgen glue; both share the root wasm + one .d.ts.
const pkg = {
  name: SCOPED_NAME,
  version,
  description:
    "DIG read-crypto for the browser and Node: URN reconstruction, AES-256-GCM-SIV decryption, and merkle inclusion-proof verification, compiled to WebAssembly. The published, installable form of the dig-client-wasm crate (roadmap #16 — stop vendoring).",
  license: "GPL-2.0-only",
  repository: { type: "git", url: REPO_URL },
  type: "module",
  // CommonJS `require` resolves to the .cjs node entry; everything else (bundlers,
  // browser, ESM import) resolves to the async-init web entry.
  main: "./node/dig_client.cjs",
  module: "./web/dig_client.js",
  types: "./dig_client.d.ts",
  exports: {
    ".": {
      // `node` is matched for both `require` and `import` under Node, so split it
      // into require (CJS .cjs) vs import (ESM web) to give each the right glue.
      node: {
        types: "./node/dig_client.d.cts",
        require: "./node/dig_client.cjs",
        import: "./web/dig_client.js",
      },
      types: "./dig_client.d.ts",
      browser: "./web/dig_client.js",
      import: "./web/dig_client.js",
      default: "./web/dig_client.js",
    },
    "./web": {
      types: "./dig_client.d.ts",
      default: "./web/dig_client.js",
    },
    "./node": {
      types: "./node/dig_client.d.cts",
      require: "./node/dig_client.cjs",
      default: "./node/dig_client.cjs",
    },
    "./dig_client_bg.wasm": "./dig_client_bg.wasm",
    "./integrity.json": "./integrity.json",
    "./package.json": "./package.json",
  },
  files: [
    "dig_client_bg.wasm",
    "dig_client.d.ts",
    "integrity.json",
    "web/dig_client.js",
    "web/dig_client.d.ts",
    "node/dig_client.cjs",
    "node/dig_client.d.cts",
    "README.md",
  ],
  // The wasm-bindgen `start` hook (install_global) is a side effect; keep glue
  // flagged so bundlers do not tree-shake the module init away.
  sideEffects: ["./web/dig_client.js", "./node/dig_client.cjs"],
  // Scoped packages default to restricted; force public so `npm publish` works.
  publishConfig: { access: "public" },
  // Mirror the wasm integrity into the manifest for quick inspection.
  digIntegrity: { sha256: wasmSha256, sri: wasmSri },
};
writeFileSync(resolve(outDir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");

// 7) README for the published package (install + integrity).
copyFileSync(resolve(root, "PUBLISHED_README.md"), resolve(outDir, "README.md"));

// 8) Final sanity gates — never leave behind a mis-publishable package.
if (pkg.name !== SCOPED_NAME) die(`name is "${pkg.name}", expected "${SCOPED_NAME}"`);
if (!/^\d+\.\d+\.\d+/.test(pkg.version)) die(`invalid version "${pkg.version}"`);

console.log(
  `assemble-pkg: pkg/ -> ${pkg.name}@${pkg.version}\n` +
    `  wasm sha256 = ${wasmSha256}\n` +
    `  wasm sri    = ${wasmSri}\n` +
    `  exports     = ., ./web, ./node, ./dig_client_bg.wasm (access: public)`
);
