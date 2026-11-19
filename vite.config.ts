import { defineConfig } from "vite";
import { readFileSync } from "node:fs";

const packageMetadata = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));
const cargoManifest = readFileSync(new URL("./Cargo.toml", import.meta.url), "utf8");
const license = cargoManifest.match(/^license\s*=\s*"([^"]+)"/m)?.[1];
if (!license) throw new Error("Application license is missing from Cargo.toml");

export default defineConfig({
  clearScreen: false,
  define: {
    __APP_VERSION__: JSON.stringify(packageMetadata.version),
    __APP_LICENSE__: JSON.stringify(license),
  },
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
