import { useState, useEffect, useCallback, useRef } from "react";
import { TitleBar } from "./TitleBar.jsx";
import { Welcome } from "./steps/Welcome.jsx";
import { License } from "./steps/License.jsx";
import { Components } from "./steps/Components.jsx";
import { Installing } from "./steps/Installing.jsx";
import { Finish } from "./steps/Finish.jsx";
import { STEPS, NOW_FILES } from "./data.jsx";
import glowD from "./assets/logos/D-glow-logo.svg";
import nebula from "./assets/logos/galaxy-background.webp";
import {
  isTauri,
  defaultInstallPath,
  pickFolder,
  runInstall,
  cancelInstall,
  launchTerminal,
  openDocs,
  copyText,
  getMeta,
  bundledDigstoreVersion,
} from "./bridge.js";

const DEFAULT_META = { version: "1.0.0", compiler: "1.0.0" };
// Fallback for the bundled digstore CLI version until the backend answers
// (matches bridge.js' browser-sim fallback and the current ship version).
const DEFAULT_DIGSTORE_VERSION = "0.3.0";

export function App() {
  // chrome: pick the OS-appropriate window controls. Windows → "win",
  // macOS → "mac", else frameless dots. (Mirrors the prototype's tweak.)
  const [chrome] = useState(() => {
    const ua = navigator.userAgent || "";
    if (/Mac/i.test(ua)) return "mac";
    if (/Win/i.test(ua)) return "win";
    return "frameless";
  });

  const [meta, setMeta] = useState(DEFAULT_META);
  // The bundled digstore CLI version this installer will install — shown on the
  // badge and the Welcome/Finish "version" chips (distinct from the installer
  // app's own version in `meta.version`).
  const [digstoreVersion, setDigstoreVersion] = useState(DEFAULT_DIGSTORE_VERSION);
  const [step, setStep] = useState(() => {
    const s = parseInt(localStorage.getItem("dig_step") || "0", 10);
    return s === 3 ? 2 : isNaN(s) ? 0 : s; // never resume mid-install
  });
  const [agreed, setAgreed] = useState(false);
  const [sel, setSel] = useState({ host: true, completions: true, path: true, example: false });
  const [installPath, setInstallPath] = useState("/usr/local/digstore");
  const [pct, setPct] = useState(0);
  const [lines, setLines] = useState([]);
  const [nowFile, setNowFile] = useState(NOW_FILES[0]);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState(null);

  const installToken = useRef(0); // bump to cancel/ignore stale install streams

  useEffect(() => {
    localStorage.setItem("dig_step", String(step));
  }, [step]);

  // Resolve the real per-OS default install path + version metadata from the backend.
  useEffect(() => {
    let alive = true;
    (async () => {
      const p = await defaultInstallPath();
      if (alive && p) setInstallPath(p);
      const m = await getMeta();
      if (alive && m) setMeta(m);
      // The badge/chips show the bundled digstore CLI version, not the app's.
      const dv = await bundledDigstoreVersion();
      if (alive && dv) setDigstoreVersion(dv);
    })();
    return () => {
      alive = false;
    };
  }, []);

  // ---- the real install (replaces the prototype's rAF animation) ----
  const startInstall = useCallback(async () => {
    const token = ++installToken.current;
    setPct(0);
    setLines([]);
    setError(null);
    setNowFile(NOW_FILES[0]);

    await runInstall(
      { installPath, selected: { cli: true, ...sel } },
      {
        onProgress: (p) => {
          if (token !== installToken.current) return;
          if (typeof p.pct === "number") setPct(p.pct);
          if (p.nowFile) setNowFile(p.nowFile);
          if (p.line) setLines((prev) => [...prev, p.line]);
        },
        onError: (err) => {
          if (token !== installToken.current) return;
          setError({ title: "Installation failed", message: err.message || String(err) });
          setLines((prev) => [
            ...prev,
            `<span class="err">✗ ${escapeHtml(err.message || String(err))}</span>`,
          ]);
        },
        onDone: () => {
          if (token !== installToken.current) return;
          setPct(100);
          setNowFile("done");
        },
      }
    );
  }, [installPath, sel]);

  useEffect(() => {
    if (step === 3) startInstall();
    // leaving step 3 cancels any in-flight stream
    return () => {
      if (step === 3) {
        installToken.current++;
        cancelInstall();
      }
    };
  }, [step, startInstall]);

  const toggle = (id) => setSel((s) => ({ ...s, [id]: !s[id] }));

  const onChangeFolder = async () => {
    const dir = await pickFolder(installPath);
    if (dir) setInstallPath(dir);
  };

  const copyCmds = async () => {
    // Displayed lines mirror the prototype verbatim for design fidelity, but the
    // clipboard payload uses the real CLI's runnable form (this build's `init`
    // takes no positional store name; the store is created in the cwd's .dig).
    await copyText("digstore init\ndigstore add ./site\ndigstore commit -m \"v1\"");
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };

  const retry = () => {
    setError(null);
    startInstall();
  };

  const installDone = step === 3 && pct >= 100 && !error;
  const canContinue = step === 1 ? agreed : step === 3 ? installDone : true;

  // Welcome/Finish "version" chips should show the bundled digstore CLI version
  // being installed, not the installer app's own version.
  const digstoreMeta = { ...meta, version: digstoreVersion };

  const primaryLabel =
    step === 0
      ? "Install DigStore"
      : step === 2
      ? "Install"
      : step === 3
      ? error
        ? "Retry"
        : installDone
        ? "Continue"
        : "Installing…"
      : step === 4
      ? "Launch Terminal"
      : "Continue";

  const go = (n) => {
    if (n >= 0 && n < STEPS.length) setStep(n);
  };

  const next = async () => {
    if (step === 3 && error) return retry();
    if (step === 4) return launchTerminal(installPath);
    if (canContinue) go(step + 1);
  };

  return (
    <div className="win">
      <TitleBar chrome={chrome} />
      <div className="body">
        {/* rail — gains `installing` while the pipeline runs so the brand glow
            intensifies with progress (--rail-pct drives the glow strength). */}
        <div
          className={"rail" + (step === 3 && !error ? " installing" : "")}
          style={{ "--rail-pct": step === 3 ? pct / 100 : 0 }}
        >
          <div className="nebula" style={{ backgroundImage: `url(${nebula})` }}></div>
          <div className="rail-top">
            <div className="bigD">
              <img src={glowD} alt="DIG" />
            </div>
            <h1>DigStore</h1>
            <div className="tagline">The content-addressable WASM store format, by DIG Network.</div>
            <div className="ver-pill">
              <span className="dot"></span>DigStore v{digstoreVersion}
            </div>
          </div>
          <div className="steps">
            {STEPS.map((s, i) => (
              <div
                key={s}
                className={"step" + (i === step ? " active" : i < step ? " done" : "")}
                onClick={() => i < step && step !== 3 && go(i)}
              >
                <span className="idx">
                  {i < step ? (
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M4 12.5l5 5L20 6.5" />
                    </svg>
                  ) : (
                    i + 1
                  )}
                </span>
                {s}
              </div>
            ))}
          </div>
          <div className="rail-foot">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="#7CE0A8" strokeWidth="2">
              <circle cx="12" cy="12" r="9" />
            </svg>
            A Proof-of-Stake Layer 2 on Chia
          </div>
        </div>

        {/* content */}
        <div className="content">
          <div className="pane" key={step}>
            {step === 0 && <Welcome meta={digstoreMeta} />}
            {step === 1 && <License agreed={agreed} setAgreed={setAgreed} />}
            {step === 2 && <Components sel={sel} toggle={toggle} path={installPath} onChange={onChangeFolder} />}
            {step === 3 && <Installing pct={pct} lines={lines} nowFile={nowFile} error={error} />}
            {step === 4 && <Finish path={installPath} onCopy={copyCmds} copied={copied} meta={digstoreMeta} />}
          </div>
          <div className="footer">
            <div className="dots">
              {STEPS.map((s, i) => (
                <span key={i} className={"d" + (i === step ? " on" : "")}></span>
              ))}
            </div>
            <div className="foot-spacer"></div>

            {/* Back: hidden on steps 0, 3, 4 */}
            {step > 0 && step !== 3 && step !== 4 && (
              <button className="btn btn-secondary" onClick={() => go(step - 1)}>
                Back
              </button>
            )}

            {/* Error state on step 3 gets a "View log" secondary action */}
            {step === 3 && error && (
              <button className="btn btn-secondary" onClick={() => openLog(lines)}>
                View log
              </button>
            )}

            {/* Done step gets the "Open Documentation" secondary */}
            {step === 4 && (
              <button className="btn btn-secondary" onClick={openDocs}>
                Open Documentation
              </button>
            )}

            <button
              className={"btn " + (step === 3 && error ? "btn-danger" : "btn-primary")}
              onClick={next}
              disabled={!canContinue && !(step === 3 && error)}
            >
              {primaryLabel}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// "View log": in Tauri we could open a temp file; for now dump the rendered
// log lines into a new window/blob so the user can read/copy the full trace.
function openLog(lines) {
  const text = lines.map((l) => l.replace(/<[^>]+>/g, "")).join("\n");
  if (isTauri()) {
    // best-effort: copy to clipboard so it's recoverable everywhere
    copyText(text);
  }
  try {
    const w = window.open("", "_blank", "width=720,height=520");
    if (w) {
      w.document.title = "DigStore install log";
      w.document.body.style.cssText = "background:#0A0A20;color:#C5C1E0;font:12.5px ui-monospace,monospace;padding:18px;white-space:pre-wrap;";
      w.document.body.textContent = text;
    }
  } catch {
    /* popups blocked — clipboard copy above is the fallback */
  }
}
