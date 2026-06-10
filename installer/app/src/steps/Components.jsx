import { Ic } from "../icons.jsx";
import { COMPONENTS } from "../data.jsx";

// Totals recompute live from selection, exactly as the prototype does:
// total download = sum of selected sizes; disk after install = total × 1.4.
export function Components({ sel, toggle, path, onChange }) {
  const total = COMPONENTS.filter((c) => c.req || sel[c.id]).reduce((s, c) => s + (parseFloat(c.size) || 0), 0);
  return (
    <div className="fade-key">
      <div className="eyebrow">Step 03 — Setup</div>
      <h2>Choose Components</h2>
      <p className="lead" style={{ marginBottom: 28 }}>
        Pick what to install and where. The CLI is required; everything else is optional.
      </p>
      <p className="field-label">Install location</p>
      <div className="path-row">
        <div className="path-input">
          {Ic.folder}
          <span>{path}</span>
        </div>
        <button className="btn-ghost" onClick={onChange}>
          Change…
        </button>
      </div>
      <p className="field-label">Components</p>
      {COMPONENTS.map((c) => {
        const on = c.req || sel[c.id];
        return (
          <div className={"comp" + (c.req ? " req" : "")} key={c.id} onClick={() => !c.req && toggle(c.id)}>
            <div className={"check" + (on ? " on" : "")} style={{ width: 22, height: 22, flex: "0 0 22px" }}>
              {Ic.check}
            </div>
            <div>
              <div className="ci">{c.name}</div>
              <div className="cd">{c.desc}</div>
            </div>
            {c.req ? <span className="pill-req">REQUIRED</span> : <span className="size">{c.size}</span>}
          </div>
        );
      })}
      <div className="meta-chips" style={{ marginTop: 22 }}>
        <span className="chip">
          <span className="k">total download</span>
          <b>~{total.toFixed(1)} MB</b>
        </span>
        <span className="chip">
          <span className="k">disk after install</span>
          <b>~{(total * 1.4).toFixed(0)} MB</b>
        </span>
      </div>
    </div>
  );
}
