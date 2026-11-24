// Fail the site build if a visual tool has no real capture or stale metadata.
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tools } from "../src/tools.js";

const directory = new URL("../public/screenshots/", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("manifest.json", directory), "utf8"));
const scenes = JSON.parse(readFileSync(new URL("scenes.json", import.meta.url), "utf8"));
// tytts is demonstrated with audio examples rather than screenshots.
const visualTools = tools.filter(tool => tool.name !== "tytts");
for (const { name } of visualTools) {
  if (!Object.values(scenes).some(scene => scene.tool === name)) {
    throw new Error(`Missing screenshot scene for ${name}`);
  }
}
for (const [name, scene] of Object.entries(scenes)) {
  const image = readFileSync(new URL(`${name}.png`, directory));
  const entry = manifest[name];
  if (!entry || image.toString("hex", 0, 8) !== "89504e470d0a1a0a"
    || entry.tool !== scene.tool || entry.caption !== scene.caption || entry.scene !== name
    || image.readUInt32BE(16) !== entry.width || image.readUInt32BE(20) !== entry.height
    || entry.width < 1 || entry.height < 1 || entry.width > 1200 || entry.height > 800
    || entry.sha256 !== createHash("sha256").update(image).digest("hex")) {
    throw new Error(`Missing or stale screenshot for ${name}; run npm run screenshots`);
  }
}
console.log(`Screenshot coverage: ${Object.keys(scenes).length} views across ${visualTools.length} tools`);
