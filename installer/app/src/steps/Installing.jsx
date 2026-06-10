import { useEffect, useRef } from "react";
import { Ic } from "../icons.jsx";

// `lines` are HTML strings emitted by the Rust pipeline (with .ok/.ac/.dim/.err
// spans). The terminal auto-scrolls as lines append. On error, the progress
// fill tints red, the caret stops, and an error banner appears (new — the
// prototype has no error state).
export function Installing({ pct, lines, nowFile, error }) {
  const termRef = useRef(null);
  useEffect(() => {
    if (termRef.current) termRef.current.scrollTop = termRef.current.scrollHeight;
  }, [lines]);

  const done = pct >= 100 && !error;
  return (
    <div className="fade-key">
      <div className="eyebrow">Step 04 — Installing</div>
      <h2>{error ? "Install failed" : done ? "Install complete" : "Installing DigStore"}</h2>
      <div className="prog-wrap">
        <div className="prog-head">
          <span className="pct">{Math.floor(pct)}%</span>
          <span className="nowfile">{error ? "stopped" : done ? "done" : "writing  " + nowFile}</span>
        </div>
        <div className="track">
          <div className={"fill" + (error ? " err" : "")} style={{ width: pct + "%" }}></div>
        </div>
        <div className="term" ref={termRef}>
          {lines.map((l, i) => (
            <div className="ln" key={i} dangerouslySetInnerHTML={{ __html: l }} />
          ))}
          {!done && !error && (
            <div className="ln caret-line">
              <span className="ac">▍</span>
            </div>
          )}
        </div>
        {error && (
          <div className="err-banner">
            <span className="eic">{Ic.alert}</span>
            <div>
              <p className="et">{error.title || "Installation error"}</p>
              <p className="em">{error.message}</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
