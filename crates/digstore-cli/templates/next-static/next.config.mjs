/** @type {import('next').NextConfig} */
const nextConfig = {
  // Emit a fully static site to `out/` — no Node server — so it can be served
  // from a DIG capsule.
  output: "export",
  // Relative asset paths so the site works from any DIG path.
  images: { unoptimized: true },
  trailingSlash: true,
};

export default nextConfig;
