/* Title bar — frameless / mac / win chrome variants from the prototype.
   The whole bar is a Tauri drag region; the controls are marked no-drag and
   wired to real window operations (minimize / maximize / close). */
import { getCurrentWindow } from "@tauri-apps/api/window";

function winOp(op) {
  try {
    const w = getCurrentWindow();
    if (op === "min") return w.minimize();
    if (op === "max") return w.toggleMaximize();
    if (op === "close") return w.close();
  } catch {
    // Running outside Tauri (plain browser preview) — ignore.
  }
}

export function TitleBar({ chrome }) {
  const title = (
    <div className="tb-title">
      <span className="tdot">
        <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="3" strokeLinecap="round">
          <path d="M5 12h14" />
        </svg>
      </span>
      DigStore Installer
    </div>
  );

  if (chrome === "mac") {
    return (
      <div className="titlebar" data-tauri-drag-region>
        <div className="ctrls mac mac-left" data-tauri-no-drag>
          <span className="c r" onClick={() => winOp("close")}></span>
          <span className="c y" onClick={() => winOp("min")}></span>
          <span className="c g" onClick={() => winOp("max")}></span>
        </div>
        <div className="tb-spacer"></div>
        {title}
        <div className="tb-spacer"></div>
      </div>
    );
  }

  if (chrome === "win") {
    return (
      <div className="titlebar" data-tauri-drag-region>
        {title}
        <div className="tb-spacer"></div>
        <div className="ctrls win" data-tauri-no-drag>
          <span className="c" onClick={() => winOp("min")}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
              <path d="M5 12h14" />
            </svg>
          </span>
          <span className="c" onClick={() => winOp("max")}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
              <rect x="5" y="5" width="14" height="14" />
            </svg>
          </span>
          <span className="c close" onClick={() => winOp("close")}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6">
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </span>
        </div>
      </div>
    );
  }

  // frameless (default)
  return (
    <div className="titlebar" data-tauri-drag-region>
      {title}
      <div className="tb-spacer"></div>
      <div className="ctrls frameless" data-tauri-no-drag>
        <span className="c" onClick={() => winOp("min")}></span>
        <span className="c" onClick={() => winOp("max")}></span>
        <span className="c close" onClick={() => winOp("close")}></span>
      </div>
    </div>
  );
}
