// Vite plugin: `import deps from "virtual:cargo-deps"` gives the Rust dependencies of every
// crate in the workspace, read from the Cargo manifests and Cargo.lock at build time.
// Nothing is committed; each build (and the Pages deploy) picks up the current versions.
import { readFileSync, readdirSync, existsSync, writeFileSync, mkdirSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { parse } from "smol-toml";

const ID = "virtual:cargo-deps";
const RESOLVED = "\0" + ID;

const SECTIONS = ["dependencies", "build-dependencies", "dev-dependencies"];

function load(root) {
  const files = [];
  const read = (path) => {
    files.push(path);
    return parse(readFileSync(path, "utf8"));
  };

  const workspace = read(resolve(root, "Cargo.toml"));
  const wsDeps = workspace.workspace?.dependencies ?? {};

  // Resolved versions from the lock file (a name can appear more than once).
  const locked = {};
  for (const pkg of read(resolve(root, "Cargo.lock")).package ?? []) {
    (locked[pkg.name] ??= []).push(pkg.version);
  }

  // When the lock holds several versions of a crate (one pulled in by something else),
  // keep the ones that satisfy this requirement under Cargo's default caret rules.
  const matching = (versions, req) => {
    if (!req || versions.length < 2) return versions;
    const want = req.replace(/^[=^~]/, "").split(".");
    const keep = want[0] === "0" ? want.slice(0, 2) : want.slice(0, 1);
    const hits = versions.filter((v) => keep.every((p, i) => v.split(".")[i] === p));
    return hits.length ? hits : versions;
  };

  const asSpec = (v) => (typeof v === "string" ? { version: v } : v ?? {});

  const crates = [];
  const cratesDir = resolve(root, "crates");
  for (const dir of readdirSync(cratesDir).sort()) {
    const manifestPath = resolve(cratesDir, dir, "Cargo.toml");
    if (!existsSync(manifestPath)) continue;
    const manifest = read(manifestPath);

    // Collect [dependencies] etc., plus [target.'cfg(...)'.dependencies].
    const tables = SECTIONS.map((kind) => ({ kind, target: null, table: manifest[kind] }));
    for (const [target, t] of Object.entries(manifest.target ?? {})) {
      for (const kind of SECTIONS) tables.push({ kind, target, table: t[kind] });
    }

    const deps = [];
    for (const { kind, target, table } of tables) {
      for (const [name, raw] of Object.entries(table ?? {})) {
        const own = asSpec(raw);
        const base = own.workspace ? asSpec(wsDeps[name]) : {};
        const pkgName = own.package ?? base.package ?? name;
        const isPath = Boolean(own.path ?? base.path);
        deps.push({
          name,
          kind,
          target,
          path: isPath,
          req: own.version ?? base.version ?? null,
          versions: matching(locked[pkgName] ?? [], own.version ?? base.version),
          features: [...(base.features ?? []), ...(own.features ?? [])],
          defaultFeatures: (own["default-features"] ?? base["default-features"]) !== false,
          optional: Boolean(own.optional),
        });
      }
    }
    deps.sort((a, b) => a.name.localeCompare(b.name));
    crates.push({ name: manifest.package?.name ?? dir, deps });
  }

  // One row per external dependency, with every crate that uses it.
  const all = {};
  for (const c of crates) {
    for (const d of c.deps) {
      if (d.path) continue;
      const row = (all[d.name] ??= { ...d, features: new Set(), usedBy: [] });
      d.features.forEach((f) => row.features.add(f));
      row.usedBy.push(c.name);
    }
  }
  const external = Object.values(all)
    .map((r) => ({ ...r, features: [...r.features].sort() }))
    .sort((a, b) => a.name.localeCompare(b.name));

  return {
    data: {
      version: workspace.workspace?.package?.version ?? null,
      crates,
      external,
      lockedPackages: Object.values(locked).reduce((n, v) => n + v.length, 0),
    },
    files,
  };
}

// Cargo.lock has no licenses, so look them up on crates.io (one request a second, as its
// crawler policy asks). Results are cached; a failed lookup just shows no license.
const CACHE = resolve(import.meta.dirname, "node_modules/.cache/cargo-licenses.json");
const USER_AGENT = "tys-tools-site (https://github.com/tylergets/tys-tools)";

async function addLicenses(external) {
  let cache = {};
  try {
    cache = JSON.parse(readFileSync(CACHE, "utf8"));
  } catch {}
  let first = true;
  for (const dep of external) {
    const key = `${dep.name}@${dep.versions[0]}`;
    if (!(key in cache) && dep.versions[0]) {
      if (!first) await new Promise((r) => setTimeout(r, 1000));
      first = false;
      try {
        const res = await fetch(`https://crates.io/api/v1/crates/${dep.name}/${dep.versions[0]}`, {
          headers: { "User-Agent": USER_AGENT },
          signal: AbortSignal.timeout(10_000),
        });
        if (res.ok) cache[key] = (await res.json()).version?.license ?? null;
      } catch {}
    }
    dep.license = cache[key] ?? null;
  }
  try {
    mkdirSync(dirname(CACHE), { recursive: true });
    writeFileSync(CACHE, JSON.stringify(cache, null, 2));
  } catch {}
}

export default function cargoDeps({ root }) {
  return {
    name: "cargo-deps",
    resolveId: (id) => (id === ID ? RESOLVED : null),
    async load(id) {
      if (id !== RESOLVED) return null;
      const { data, files } = load(root);
      await addLicenses(data.external);
      files.forEach((f) => this.addWatchFile(f));
      return `export default ${JSON.stringify(data)};`;
    },
  };
}
