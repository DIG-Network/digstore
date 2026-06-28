export const metadata = {
  title: "My DIG Next app",
  description: "A statically-exported Next.js app served from DIG.",
};

export default function RootLayout({ children }) {
  return (
    <html lang="en">
      <body
        style={{
          margin: 0,
          background: "#0c0f14",
          color: "#e7ecf3",
          fontFamily: "system-ui, sans-serif",
        }}
      >
        {children}
      </body>
    </html>
  );
}
