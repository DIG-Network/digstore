/* DigStore Installer — clickable wizard. Vanilla-React via Babel. */
const { useState, useEffect, useRef, useCallback } = React;

/* ---------- icons ---------- */
const Ic = {
  git:   <svg viewBox="0 0 24 24" fill="none" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="5" r="2.4"/><circle cx="12" cy="19" r="2.4"/><circle cx="19" cy="12" r="2.4"/><path d="M12 7.4v9.2M14.4 12h2.2M12 12c0-2.6 2.2-4.6 5-4.6"/></svg>,
  lock:  <svg viewBox="0 0 24 24" fill="none" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="4" y="10" width="16" height="10" rx="2.5"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/><circle cx="12" cy="15" r="1.4"/></svg>,
  shield:<svg viewBox="0 0 24 24" fill="none" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M12 3l7 3v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6z"/><path d="M9 12l2 2 4-4.5"/></svg>,
  check: <svg viewBox="0 0 24 24" fill="none" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"><path d="M4 12.5l5 5L20 6.5"/></svg>,
  folder:<svg viewBox="0 0 24 24" fill="none" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4l2 2.5h9A1.5 1.5 0 0 1 21 9v9.5A1.5 1.5 0 0 1 19.5 20h-15A1.5 1.5 0 0 1 3 18.5z"/></svg>,
  copy:  <svg viewBox="0 0 24 24" fill="none" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h8"/></svg>,
};

const STEPS = ["Welcome", "License", "Components", "Install", "Done"];

const FEATURES = [
  { ic: Ic.git, h: "A Git-shaped workflow", p: "init, add, commit, log, diff, checkout, clone — the verbs you already know. Chunking, encryption and WASM compilation stay under the surface." },
  { ic: Ic.lock, h: "Encrypted at rest, by URN", p: "Every URN is a key. Content is chunked, SHA-256 addressed, and sealed with an AES-256-GCM key derived from the URN itself." },
  { ic: Ic.shield, h: "Provable & secretless", p: "Each store compiles to one portable .wasm that defends itself — merkle proofs, host attestation, and zero-knowledge proofs of execution. No embedded secret to extract." },
];

const COMPONENTS = [
  { id: "cli", name: "DigStore CLI", desc: "The digstore command — init, add, commit, log, clone.", size: "18.4 MB", req: true },
  { id: "host", name: "Host Runtime", desc: "Sandboxed WASM host with attestation + session ABI.", size: "21.0 MB", on: true },
  { id: "completions", name: "Shell completions", desc: "bash · zsh · fish tab-completion for digstore.", size: "0.3 MB", on: true },
  { id: "path", name: "Add digstore to PATH", desc: "Symlink digstore into /usr/local/bin.", size: "—", on: true },
  { id: "example", name: "Example store", desc: "A sample urn:dig store to clone and explore.", size: "6.1 MB", on: false },
];

const INSTALL_LOG = [
  { t: 240, html: '<span class="dim">$</span> digstore-setup --target /usr/local/digstore' },
  { t: 520, html: 'Resolving release <span class="ac">v1.0.0</span> · compiler 1.0.0 · module format 1' },
  { t: 900, html: '<span class="ok">✓</span> Verified package signature <span class="dim">(BLS · 96 bytes)</span>' },
  { t: 1300, html: 'Unpacking <span class="ac">DigStore CLI</span> → /usr/local/digstore/bin' },
  { t: 1750, html: 'Unpacking <span class="ac">Host Runtime</span> <span class="dim">(64 KiB → 16 MiB memory bounds)</span>' },
  { t: 2200, html: 'Embedding trusted host keys <span class="dim">dig-host-key-v1:…</span>' },
  { t: 2650, html: '<span class="ok">✓</span> Content-defined chunking ready <span class="dim">(16/64/256 KiB)</span>' },
  { t: 3050, html: 'Linking <span class="ac">digstore</span> → /usr/local/bin/digstore' },
  { t: 3450, html: 'Installing shell completions <span class="dim">bash · zsh · fish</span>' },
  { t: 3850, html: '<span class="ok">✓</span> Verifying install · merkle root committed' },
  { t: 4150, html: '<span class="ok">✓</span> DigStore is ready.' },
];
const NOW_FILES = ["bin/digstore", "lib/dig_host.wasm", "lib/compiler.wasm", "share/completions/_digstore", "trusted/host-keys.toml", "examples/hello.wasm"];

/* ===================== title bar ===================== */
function TitleBar({ chrome }) {
  const title = (
    <div className="tb-title">
      <span className="tdot"><svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="3" strokeLinecap="round"><path d="M5 12h14"/></svg></span>
      DigStore Installer
    </div>
  );
  if (chrome === "mac") {
    return (
      <div className="titlebar">
        <div className="ctrls mac mac-left"><span className="c r"></span><span className="c y"></span><span className="c g"></span></div>
        <div className="tb-spacer"></div>{title}<div className="tb-spacer"></div>
      </div>
    );
  }
  if (chrome === "win") {
    return (
      <div className="titlebar">
        {title}<div className="tb-spacer"></div>
        <div className="ctrls win">
          <span className="c"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><path d="M5 12h14"/></svg></span>
          <span className="c"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><rect x="5" y="5" width="14" height="14"/></svg></span>
          <span className="c close"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><path d="M6 6l12 12M18 6L6 18"/></svg></span>
        </div>
      </div>
    );
  }
  return (
    <div className="titlebar">
      {title}<div className="tb-spacer"></div>
      <div className="ctrls frameless"><span className="c"></span><span className="c"></span><span className="c close"></span></div>
    </div>
  );
}

/* ===================== steps ===================== */
function Welcome() {
  return (
    <div className="fade-key">
      <div className="eyebrow">DigStore CLI · Host Runtime</div>
      <h2>Install <span className="gt">DigStore</span></h2>
      <p className="lead">The content-addressable WASM store format. Your content and the logic that serves it compile into one portable, encrypted, self-defending executable.</p>
      <div className="feats">
        {FEATURES.map((f, i) => (
          <div className="feat" key={i}>
            <div className="ic">{f.ic}</div>
            <div><h4>{f.h}</h4><p>{f.p}</p></div>
          </div>
        ))}
      </div>
      <div className="meta-chips">
        <span className="chip"><span className="k">version</span><b>1.0.0</b></span>
        <span className="chip"><span className="k">install size</span><b>~46 MB</b></span>
        <span className="chip"><span className="k">platforms</span>macOS · Linux · Windows</span>
        <span className="chip"><span className="k">license</span>Apache-2.0</span>
      </div>
    </div>
  );
}

function License({ agreed, setAgreed }) {
  return (
    <div className="fade-key">
      <div className="eyebrow">Step 02 — Terms</div>
      <h2>License Agreement</h2>
      <p className="lead">Review the terms below. DigStore is open source under the Apache License 2.0.</p>
      <div className="license">
        <h5>DigStore — End User License &amp; Apache License 2.0</h5>
        <p className="muted">Version 1.0 · The Content-Addressable WASM Store Format · © 2026 DIG Network</p>
        <p>1. GRANT. Subject to the terms of the Apache License, Version 2.0 (the "License"), you are granted a perpetual, worldwide, non-exclusive, royalty-free license to use, reproduce and distribute the DigStore command-line interface and host runtime (the "Software").</p>
        <p>2. THE MODULE IS THE ARTIFACT. A DigStore store compiles to a single WebAssembly module that embeds its own content, merkle commitments, root history, store public key and trusted-host keys. The module embeds no secret of any kind. You acknowledge that content is gated by the module itself, not by this Software.</p>
        <p>3. URN AS CREDENTIAL. Content is addressed by URNs of the form urn:dig:&lt;chain&gt;:&lt;storeID&gt;[:&lt;rootHash&gt;][/&lt;resourceKey&gt;]. The retrieval key and decryption key are derived from the URN and nothing else. You are solely responsible for the distribution of, and access to, any URN you hold or publish.</p>
        <p>4. PROVIDER BLINDNESS. A provider serving a module receives a retrieval hash and returns ciphertext. Decryption runs on the client. The DIG Network operates as a neutral pipe by construction and disclaims knowledge of relayed content.</p>
        <p>5. WARRANTY. The Software is provided "AS IS", WITHOUT WARRANTY OF ANY KIND, express or implied, including but not limited to the warranties of merchantability and fitness for a particular purpose.</p>
        <p>6. LIMITATION OF LIABILITY. In no event shall the authors or copyright holders be liable for any claim, damages or other liability arising from, out of or in connection with the Software or its use.</p>
        <p className="muted">Full text: apache.org/licenses/LICENSE-2.0 — scroll reviewed.</p>
      </div>
      <div className="agree" onClick={() => setAgreed(!agreed)}>
        <div className={"check" + (agreed ? " on" : "")}>{Ic.check}</div>
        <span>I have read and agree to the DigStore License Agreement.</span>
      </div>
    </div>
  );
}

function Components({ sel, toggle, path }) {
  const total = COMPONENTS.filter(c => c.req || sel[c.id]).reduce((s, c) => s + (parseFloat(c.size) || 0), 0);
  return (
    <div className="fade-key">
      <div className="eyebrow">Step 03 — Setup</div>
      <h2>Choose Components</h2>
      <p className="lead" style={{ marginBottom: 28 }}>Pick what to install and where. The CLI is required; everything else is optional.</p>
      <p className="field-label">Install location</p>
      <div className="path-row">
        <div className="path-input">{Ic.folder}<span>{path}</span></div>
        <button className="btn-ghost">Change…</button>
      </div>
      <p className="field-label">Components</p>
      {COMPONENTS.map(c => {
        const on = c.req || sel[c.id];
        return (
          <div className={"comp" + (c.req ? " req" : "")} key={c.id} onClick={() => !c.req && toggle(c.id)}>
            <div className={"check" + (on ? " on" : "")} style={{ width: 22, height: 22, flex: "0 0 22px" }}>{Ic.check}</div>
            <div><div className="ci">{c.name}</div><div className="cd">{c.desc}</div></div>
            {c.req ? <span className="pill-req">REQUIRED</span> : <span className="size">{c.size}</span>}
          </div>
        );
      })}
      <div className="meta-chips" style={{ marginTop: 22 }}>
        <span className="chip"><span className="k">total download</span><b>~{total.toFixed(1)} MB</b></span>
        <span className="chip"><span className="k">disk after install</span><b>~{(total * 1.4).toFixed(0)} MB</b></span>
      </div>
    </div>
  );
}

function Installing({ pct, lines, nowFile }) {
  const termRef = useRef(null);
  useEffect(() => { if (termRef.current) termRef.current.scrollTop = termRef.current.scrollHeight; }, [lines]);
  return (
    <div className="fade-key">
      <div className="eyebrow">Step 04 — Installing</div>
      <h2>{pct >= 100 ? "Install complete" : "Installing DigStore"}</h2>
      <div className="prog-wrap">
        <div className="prog-head">
          <span className="pct">{Math.floor(pct)}%</span>
          <span className="nowfile">{pct >= 100 ? "done" : "writing  " + nowFile}</span>
        </div>
        <div className="track"><div className="fill" style={{ width: pct + "%" }}></div></div>
        <div className="term" ref={termRef}>
          {lines.map((l, i) => <div className="ln" key={i} dangerouslySetInnerHTML={{ __html: l }} />)}
          {pct < 100 && <div className="ln"><span className="ac">▍</span></div>}
        </div>
      </div>
    </div>
  );
}

function Finish({ path, onCopy, copied }) {
  return (
    <div className="fade-key finish">
      <div className="seal"><div className="ring"></div>{Ic.check}</div>
      <h2>DigStore is <span className="gt">installed</span></h2>
      <p className="lead">The CLI and host runtime are ready. Initialize your first content-addressable store and commit a generation.</p>
      <div className="recap">
        <span className="chip"><span className="k">version</span><b>1.0.0</b></span>
        <span className="chip"><span className="k">location</span>{path}</span>
        <span className="chip"><span className="dot" style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--ok)", display: "inline-block" }}></span>digstore on PATH</span>
      </div>
      <div className="next">
        <div className="nh">Next steps</div>
        <div className="cmd">
          <button className="copy" onClick={onCopy}>{copied ? Ic.check : Ic.copy}{copied ? "Copied" : "Copy"}</button>
          <div className="c-line"><span className="p">$</span> <span className="cmd-t">digstore init my-store</span>   <span className="cc"># create a store</span></div>
          <div className="c-line"><span className="p">$</span> <span className="cmd-t">digstore add ./site</span>      <span className="cc"># stage content</span></div>
          <div className="c-line"><span className="p">$</span> <span className="cmd-t">digstore commit -m "v1"</span>  <span className="cc"># compile a .wasm generation</span></div>
        </div>
      </div>
    </div>
  );
}

/* ===================== app ===================== */
const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "accent": ["#5800D6", "#FF00DE"],
  "chrome": "frameless",
  "nebula": true,
  "density": "regular"
}/*EDITMODE-END*/;

const DENSITY = {
  compact: { "--pad-x": "40px", "--pad-y": "32px", "--row-pad": "12px", "--win-radius": "14px" },
  regular: { "--pad-x": "56px", "--pad-y": "44px", "--row-pad": "16px", "--win-radius": "18px" },
  comfy:   { "--pad-x": "72px", "--pad-y": "56px", "--row-pad": "20px", "--win-radius": "22px" },
};

function App() {
  const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [step, setStep] = useState(() => {
    const s = parseInt(localStorage.getItem("dig_step") || "0", 10);
    return s === 3 ? 2 : (isNaN(s) ? 0 : s); // never resume mid-install
  });
  const [agreed, setAgreed] = useState(false);
  const [sel, setSel] = useState({ host: true, completions: true, path: true, example: false });
  const [pct, setPct] = useState(0);
  const [lines, setLines] = useState([]);
  const [nowFile, setNowFile] = useState(NOW_FILES[0]);
  const [copied, setCopied] = useState(false);
  const path = "/usr/local/digstore";

  useEffect(() => { localStorage.setItem("dig_step", String(step)); }, [step]);

  // apply tweaks to :root
  useEffect(() => {
    const r = document.documentElement.style;
    const [a, b] = t.accent || ["#5800D6", "#FF00DE"];
    r.setProperty("--accent-a", a); r.setProperty("--accent-b", b);
    r.setProperty("--bg-img-opacity", t.nebula ? ".6" : "0");
    const d = DENSITY[t.density] || DENSITY.regular;
    Object.entries(d).forEach(([k, v]) => r.setProperty(k, v));
  }, [t.accent, t.nebula, t.density]);

  // install animation
  const runInstall = useCallback(() => {
    setPct(0); setLines([]);
    const timers = [];
    INSTALL_LOG.forEach(ev => timers.push(setTimeout(() => setLines(p => [...p, ev.html]), ev.t)));
    const start = performance.now(), dur = 4150;
    let raf;
    const tick = (now) => {
      const p = Math.min(100, ((now - start) / dur) * 100);
      setPct(p);
      setNowFile(NOW_FILES[Math.min(NOW_FILES.length - 1, Math.floor(p / 100 * NOW_FILES.length))]);
      if (p < 100) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => { timers.forEach(clearTimeout); cancelAnimationFrame(raf); };
  }, []);

  useEffect(() => {
    if (step === 3) { const stop = runInstall(); return stop; }
  }, [step, runInstall]);

  const toggle = (id) => setSel(s => ({ ...s, [id]: !s[id] }));
  const copyCmds = () => {
    navigator.clipboard && navigator.clipboard.writeText('digstore init my-store\ndigstore add ./site\ndigstore commit -m "v1"');
    setCopied(true); setTimeout(() => setCopied(false), 1600);
  };

  const installDone = step === 3 && pct >= 100;
  const canContinue = step === 1 ? agreed : (step === 3 ? installDone : true);
  const primaryLabel = step === 0 ? "Install DigStore" : step === 2 ? "Install" : step === 3 ? (installDone ? "Continue" : "Installing…") : step === 4 ? "Launch Terminal" : "Continue";

  const go = (n) => { if (n >= 0 && n < STEPS.length) setStep(n); };
  const next = () => { if (step === 4) { setStep(0); setAgreed(false); } else if (canContinue) go(step + 1); };

  return (
    <div className="win">
      <TitleBar chrome={t.chrome} />
      <div className="body">
        {/* rail */}
        <div className="rail">
          <div className="nebula"></div>
          <div className="rail-top">
            <div className="bigD"><img src="installer/assets/D-glow-logo.svg" alt="DIG" /></div>
            <h1>DigStore</h1>
            <div className="tagline">The content-addressable WASM store format, by DIG Network.</div>
            <div className="ver-pill"><span className="dot"></span>v1.0.0 · compiler 1.0.0</div>
          </div>
          <div className="steps">
            {STEPS.map((s, i) => (
              <div key={s} className={"step" + (i === step ? " active" : i < step ? " done" : "")}
                   onClick={() => i < step && step !== 3 && go(i)}>
                <span className="idx">{i < step ? <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round"><path d="M4 12.5l5 5L20 6.5"/></svg> : i + 1}</span>
                {s}
              </div>
            ))}
          </div>
          <div className="rail-foot">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="#7CE0A8" strokeWidth="2"><circle cx="12" cy="12" r="9"/></svg>
            A Proof-of-Stake Layer 2 on Chia
          </div>
        </div>
        {/* content */}
        <div className="content">
          <div className="pane" key={step}>
            {step === 0 && <Welcome />}
            {step === 1 && <License agreed={agreed} setAgreed={setAgreed} />}
            {step === 2 && <Components sel={sel} toggle={toggle} path={path} />}
            {step === 3 && <Installing pct={pct} lines={lines} nowFile={nowFile} />}
            {step === 4 && <Finish path={path} onCopy={copyCmds} copied={copied} />}
          </div>
          <div className="footer">
            <div className="dots">{STEPS.map((s, i) => <span key={i} className={"d" + (i === step ? " on" : "")}></span>)}</div>
            <div className="foot-spacer"></div>
            {step > 0 && step !== 3 && step !== 4 && (
              <button className="btn btn-secondary" onClick={() => go(step - 1)}>Back</button>
            )}
            {step === 4 && <button className="btn btn-secondary" onClick={() => go(0)}>Open Documentation</button>}
            <button className="btn btn-primary" onClick={next} disabled={!canContinue}>{primaryLabel}</button>
          </div>
        </div>
      </div>

      <TweaksPanel>
        <TweakSection label="Brand accent" />
        <TweakColor label="Gradient" value={t.accent}
          options={[["#5800D6", "#FF00DE"], ["#5800D6", "#7CC0FF"], ["#7A1FE0", "#FF3DF5"], ["#0AB39C", "#5800D6"]]}
          onChange={(v) => setTweak("accent", v)} />
        <TweakSection label="Window" />
        <TweakRadio label="Chrome" value={t.chrome} options={["frameless", "mac", "win"]}
          onChange={(v) => setTweak("chrome", v)} />
        <TweakToggle label="Cosmic nebula" value={t.nebula} onChange={(v) => setTweak("nebula", v)} />
        <TweakSection label="Layout" />
        <TweakRadio label="Density" value={t.density} options={["compact", "regular", "comfy"]}
          onChange={(v) => setTweak("density", v)} />
      </TweaksPanel>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
