import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The WASM package under web/pkg lives outside the frontend root; allow Vite's
// dev server to serve it. The port is pinned so Tauri's devUrl stays stable.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 1420,
    strictPort: true,
    fs: { allow: [".."] },
  },
});
