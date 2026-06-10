import { Ic } from "../icons.jsx";

export function Finish({ path, onCopy, copied, meta }) {
  return (
    <div className="fade-key finish">
      <div className="seal">
        <div className="ring"></div>
        {Ic.check}
      </div>
      <h2>
        DigStore is <span className="gt">installed</span>
      </h2>
      <p className="lead">
        The CLI and host runtime are ready. Initialize your first content-addressable store and commit a generation.
      </p>
      <div className="recap">
        <span className="chip">
          <span className="k">version</span>
          <b>{meta.version}</b>
        </span>
        <span className="chip">
          <span className="k">location</span>
          {path}
        </span>
        <span className="chip">
          <span
            className="dot"
            style={{ width: 6, height: 6, borderRadius: "50%", background: "var(--ok)", display: "inline-block" }}
          ></span>
          digstore on PATH
        </span>
      </div>
      <div className="next">
        <div className="nh">Next steps</div>
        <div className="cmd">
          <button className="copy" onClick={onCopy}>
            {copied ? Ic.check : Ic.copy}
            {copied ? "Copied" : "Copy"}
          </button>
          <div className="c-line">
            <span className="p">$</span> <span className="cmd-t">digstore init my-store</span>{"   "}
            <span className="cc"># create a store</span>
          </div>
          <div className="c-line">
            <span className="p">$</span> <span className="cmd-t">digstore add ./site</span>{"      "}
            <span className="cc"># stage content</span>
          </div>
          <div className="c-line">
            <span className="p">$</span> <span className="cmd-t">digstore commit -m "v1"</span>{"  "}
            <span className="cc"># compile a .wasm generation</span>
          </div>
        </div>
      </div>
    </div>
  );
}
