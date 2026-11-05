import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath } from "node:url";
import cargoDeps from "./cargo-deps.js";

export default defineConfig({
  // cargoDeps reads the workspace manifests and Cargo.lock for the /deps page.
  plugins: [vue(), cargoDeps({ root: fileURLToPath(new URL("..", import.meta.url)) })],
  // Icons are read from crates/ty-common/icons, outside the site root.
  server: { fs: { allow: [".."] } },
});
