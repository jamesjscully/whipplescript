import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid({ jsxImportSource: "@solidjs/web" })],
  resolve: {
    alias: {
      "solid-js/web": "@solidjs/web"
    }
  },
  server: {
    port: 5177,
    strictPort: false
  }
});
