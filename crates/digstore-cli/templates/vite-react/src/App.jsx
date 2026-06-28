import { useState } from "react";

// A React app wired to the in-page Chia wallet (`window.chia`). Under
// `digstore dev` a dev shim provides window.chia so you can build the flow with
// no real wallet; in production a real wallet injects it.
export default function App() {
  const [status, setStatus] = useState(null);

  async function connect() {
    const chia = window.chia;
    if (!chia) {
      setStatus("No Chia wallet found (open in the DIG Browser or a wallet).");
      return;
    }
    try {
      if (typeof chia.connect === "function") await chia.connect();
      const chainId = await chia.request({ method: "chainId" });
      setStatus(`Connected · chainId: ${chainId}`);
    } catch (err) {
      setStatus("Wallet error: " + (err?.message ?? String(err)));
    }
  }

  return (
    <main style={styles.main}>
      <h1 style={styles.h1}>DIG + React</h1>
      <p style={styles.p}>
        Built with Vite, served through the real DIG read path by{" "}
        <code style={styles.code}>digstore dev</code>. Edit{" "}
        <code style={styles.code}>src/App.jsx</code> and save.
      </p>
      <button style={styles.button} onClick={connect}>
        Connect wallet
      </button>
      {status && <pre style={styles.pre}>{status}</pre>}
    </main>
  );
}

const styles = {
  main: {
    maxWidth: "40rem",
    margin: "0 auto",
    padding: "3rem 2rem",
    color: "#e7ecf3",
    fontFamily: "system-ui, sans-serif",
  },
  h1: { fontSize: "2.5rem", letterSpacing: "-0.02em" },
  p: { color: "#9aa7b8", lineHeight: 1.6 },
  code: { color: "#5eead4" },
  button: {
    marginTop: "1rem",
    padding: "0.7em 1.4em",
    fontWeight: 600,
    color: "#06231d",
    background: "#5eead4",
    border: 0,
    borderRadius: "0.5em",
    cursor: "pointer",
  },
  pre: {
    marginTop: "1.5rem",
    padding: "1rem",
    background: "#11181f",
    borderRadius: "0.5em",
  },
};
