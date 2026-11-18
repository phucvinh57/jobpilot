// Builds the extension into `extension/dist/` (load it unpacked from chrome://extensions):
//   bun extension/build.ts
// Bundles the service worker and the popup, and copies the manifest, the popup page and the icons.

import { copyFile, mkdir, rm } from "node:fs/promises";
import { join } from "node:path";

const root = import.meta.dir;
const out = join(root, "dist");
const icons = join(root, "..", "assets", "icons");
const iconFiles = ["jobpilot-16.png", "jobpilot-32.png", "jobpilot-48.png", "jobpilot-128.png"];

await rm(out, { recursive: true, force: true });
await mkdir(join(out, "icons"), { recursive: true });

const result = await Bun.build({
  entrypoints: [join(root, "src", "background.ts"), join(root, "src", "popup.ts")],
  outdir: out,
  target: "browser",
  format: "esm",
  splitting: false,
  minify: false,
  sourcemap: "none",
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}

await Promise.all([
  copyFile(join(root, "manifest.json"), join(out, "manifest.json")),
  copyFile(join(root, "popup.html"), join(out, "popup.html")),
  copyFile(join(root, "popup.css"), join(out, "popup.css")),
  ...iconFiles.map((file) => copyFile(join(icons, file), join(out, "icons", file))),
]);

console.log(`extension built: ${result.outputs.map((o) => o.path.slice(out.length + 1)).join(", ")} -> ${out}`);
