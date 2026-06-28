export default function Home() {
  return (
    <main style={{ maxWidth: "40rem", margin: "0 auto", padding: "3rem 2rem" }}>
      <h1 style={{ fontSize: "2.5rem", letterSpacing: "-0.02em" }}>
        DIG + Next.js
      </h1>
      <p style={{ color: "#9aa7b8", lineHeight: 1.6 }}>
        A statically-exported Next.js site, served through the real DIG read
        path. Edit <code style={{ color: "#5eead4" }}>app/page.jsx</code> and
        save.
      </p>
      <p style={{ color: "#9aa7b8", lineHeight: 1.6 }}>
        Add wallet calls via <code style={{ color: "#5eead4" }}>window.chia</code>{" "}
        in a client component. Run <code style={{ color: "#5eead4" }}>digstore deploy</code>{" "}
        to publish it on Chia.
      </p>
    </main>
  );
}
