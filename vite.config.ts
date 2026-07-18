import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The WASM package under web/pkg lives outside the frontend root; allow Vite's
// dev server to serve it.
export default defineConfig({
  plugins: [react()],
  server: {
    fs: { allow: [".."] },
  },
});
