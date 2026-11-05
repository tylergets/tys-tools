import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  // Icons are read from crates/ty-common/icons, outside the site root.
  server: { fs: { allow: [".."] } },
});
