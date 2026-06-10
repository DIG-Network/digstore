import { FEATURES } from "../data.jsx";

export function Welcome({ meta }) {
  return (
    <div className="fade-key">
      <div className="eyebrow">DigStore CLI · Host Runtime</div>
      <h2>
        Install <span className="gt">DigStore</span>
      </h2>
      <p className="lead">
        The content-addressable WASM store format. Your content and the logic that serves it compile into one portable,
        encrypted, self-defending executable.
      </p>
      <div className="feats">
        {FEATURES.map((f, i) => (
          <div className="feat" key={i}>
            <div className="ic">{f.ic}</div>
            <div>
              <h4>{f.h}</h4>
              <p>{f.p}</p>
            </div>
          </div>
        ))}
      </div>
      <div className="meta-chips">
        <span className="chip">
          <span className="k">version</span>
          <b>{meta.version}</b>
        </span>
        <span className="chip">
          <span className="k">install size</span>
          <b>~46 MB</b>
        </span>
        <span className="chip">
          <span className="k">platforms</span>macOS · Linux · Windows
        </span>
        <span className="chip">
          <span className="k">license</span>Apache-2.0
        </span>
      </div>
    </div>
  );
}
