/* Stage the prebuilt digstore binary into resources/bin/ and write its SHA-256
 * sidecar, so the installer bundles a real, verifiable artifact.
 *
 * Usage:
 *   node scripts/stage-binary.mjs [--src <path-to-digstore[.exe]>]
 *
 * Default source is the workspace release build:
 *   <repo>/target/release/digstore[.exe]
 *
 * This does NOT trigger a cargo build — it only copies an already-built binary.
 */
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, copyFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(__dirname, "..");
const repoRoot = resolve(appDir, "..", "..");

const isWin = process.platform === "win32";
const binName = isWin ? "digstore.exe" : "digstore";

// parse --src
const args = process.argv.slice(2);
let src = null;
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--src") src = args[++i];
}
if (!src) src = join(repoRoot, "target", "release", binName);
src = resolve(src);

if (!existsSync(src)) {
  console.error(`[stage-binary] source not found: ${src}`);
  console.error(`[stage-binary] build it first: cargo build -p digstore-cli --release`);
  process.exit(1);
}

const destDir = join(appDir, "src-tauri", "resources", "bin");
mkdirSync(destDir, { recursive: true });
const dest = join(destDir, binName);
copyFileSync(src, dest);

const digest = createHash("sha256").update(readFileSync(dest)).digest("hex");
writeFileSync(`${dest}.sha256`, digest + "\n");

console.log(`[stage-binary] staged ${binName} (${(readFileSync(dest).length / 1e6).toFixed(1)} MB)`);
console.log(`[stage-binary] sha256 ${digest}`);
