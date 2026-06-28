// A minimal window.chia usage example.
//
// `window.chia` is the in-page Chia wallet provider (CHIP-0002 / the DIG Browser's
// injected provider). Your dapp calls it to connect and to request signatures —
// you never handle the user's keys.
//
// Under `digstore dev` a DEV shim provides `window.chia` so you can build the flow
// with no real wallet. In production a real wallet injects the genuine provider.

const out = document.getElementById("out");
const connectBtn = document.getElementById("connect");

function show(obj) {
  out.hidden = false;
  out.textContent = typeof obj === "string" ? obj : JSON.stringify(obj, null, 2);
}

async function connect() {
  const chia = window.chia;
  if (!chia) {
    show(
      "No Chia wallet found. Open this dapp in the DIG Browser or a wallet that " +
        "injects window.chia — or run it under `digstore dev` for a dev shim.",
    );
    return;
  }

  try {
    // 1. Establish a connection / session with the wallet.
    if (typeof chia.connect === "function") {
      await chia.connect();
    }

    // 2. Ask the wallet to run a CHIP-0002 method. `request({ method, params })`
    //    is the canonical entry point; `chainId` returns the active network.
    const chainId = await chia.request({ method: "chainId" });

    show({
      connected: true,
      chainId,
      note: "Replace this with your dapp's real wallet calls (getPublicKeys, signMessageByAddress, takeOffer, …).",
    });
  } catch (err) {
    show("Wallet error: " + (err && err.message ? err.message : String(err)));
  }
}

connectBtn.addEventListener("click", connect);
