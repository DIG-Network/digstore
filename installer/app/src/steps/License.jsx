import { Ic } from "../icons.jsx";

// License summary with the DigStore-specific notes (module-is-the-artifact,
// URN-as-credential, provider-blindness). DigStore is free software under the
// GNU General Public License v2.0 — the same license as Git.
export function License({ agreed, setAgreed }) {
  return (
    <div className="fade-key">
      <div className="eyebrow">Step 02 — Terms</div>
      <h2>License Agreement</h2>
      <p className="lead">Review the terms below. DigStore is free software under the GNU General Public License v2.0 — the same license as Git.</p>
      <div className="license">
        <h5>DigStore — GNU General Public License v2.0</h5>
        <p className="muted">The Content-Addressable WASM Store Format · © 2026 DIG Network</p>
        <p>
          1. GRANT. DigStore is free software: under the GNU General Public License, version 2 (the "License"), you may
          use, study, share and modify the DigStore command-line interface and host runtime (the "Software"). When you
          distribute the Software or derivative works, you must pass on the same freedoms under the GPLv2 and make the
          corresponding source available.
        </p>
        <p>
          2. THE MODULE IS THE ARTIFACT. A DigStore store compiles to a single WebAssembly module that embeds its own
          content, merkle commitments, root history, store public key and trusted-host keys. The module embeds no secret
          of any kind. You acknowledge that content is gated by the module itself, not by this Software.
        </p>
        <p>
          3. URN AS CREDENTIAL. Content is addressed by URNs of the form
          urn:dig:&lt;chain&gt;:&lt;storeID&gt;[:&lt;rootHash&gt;][/&lt;resourceKey&gt;]. The retrieval key and decryption
          key are derived from the URN and nothing else. You are solely responsible for the distribution of, and access
          to, any URN you hold or publish.
        </p>
        <p>
          4. PROVIDER BLINDNESS. A provider serving a module receives a retrieval hash and returns ciphertext. Decryption
          runs on the client. The DIG Network operates as a neutral pipe by construction and disclaims knowledge of
          relayed content.
        </p>
        <p>
          5. WARRANTY. The Software is provided "AS IS", WITHOUT WARRANTY OF ANY KIND, express or implied, including but
          not limited to the warranties of merchantability and fitness for a particular purpose.
        </p>
        <p>
          6. LIMITATION OF LIABILITY. In no event shall the authors or copyright holders be liable for any claim, damages
          or other liability arising from, out of or in connection with the Software or its use.
        </p>
        <p className="muted">Full text: gnu.org/licenses/old-licenses/gpl-2.0.html — scroll reviewed.</p>
      </div>
      <div className="agree" onClick={() => setAgreed(!agreed)}>
        <div className={"check" + (agreed ? " on" : "")}>{Ic.check}</div>
        <span>I have read and agree to the DigStore License Agreement.</span>
      </div>
    </div>
  );
}
