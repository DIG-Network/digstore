import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Relative base so the built site works from any DIG path (no fixed host root).
export default defineConfig({
  base: "./",
  plugins: [react()],
});
