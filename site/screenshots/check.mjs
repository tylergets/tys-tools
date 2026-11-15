// Fail the site build if a listed tool has no real capture or stale metadata.
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tools } from "../src/tools.js";

const directory = new URL("../public/screenshots/", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("manifest.json", directory), "utf8"));
for (const { name } of tools) {
  const image = readFileSync(new URL(`${name}.png`, directory));
  const entry = manifest[name];
  if (!entry || image.toString("hex", 0, 8) !== "89504e470d0a1a0a"
    || image.readUInt32BE(16) !== 1200 || image.readUInt32BE(20) !== 800
    || entry.sha256 !== createHash("sha256").update(image).digest("hex")) {
    throw new Error(`Missing or stale screenshot for ${name}; run npm run screenshots`);
  }
}
console.log(`Screenshot coverage: ${tools.length} tools`);
