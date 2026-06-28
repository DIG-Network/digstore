// NFT drop page starter.
//
// Renders a small preview grid and a mint button. The mint button is wired to
// `window.chia` (the in-page Chia wallet provider) so collectors sign the mint
// with their own wallet — under `digstore dev` a dev shim provides it.

const COLLECTION = [
  { name: "Genesis #001", traits: "Aurora · Rare" },
  { name: "Genesis #002", traits: "Ember · Common" },
  { name: "Genesis #003", traits: "Tidal · Epic" },
];

const grid = document.getElementById("grid");
for (const item of COLLECTION) {
  const card = document.createElement("div");
  card.className = "card";
  card.innerHTML =
    '<div class="art"></div>' +
    "<strong>" +
    item.name +
    "</strong>" +
    '<span class="traits">' +
    item.traits +
    "</span>";
  grid.appendChild(card);
}

const mintBtn = document.getElementById("mint");
mintBtn.addEventListener("click", async () => {
  const chia = window.chia;
  if (!chia) {
    mintBtn.textContent = "No wallet found (open in the DIG Browser)";
    return;
  }
  try {
    if (typeof chia.connect === "function") await chia.connect();
    // TODO: build + submit your collection's mint spend here, signed by the
    // wallet. See the DIG docs: "Mint an NFT collection".
    mintBtn.textContent = "Wallet connected — wire up your mint spend";
  } catch (err) {
    mintBtn.textContent = "Wallet error: " + (err && err.message ? err.message : err);
  }
});
