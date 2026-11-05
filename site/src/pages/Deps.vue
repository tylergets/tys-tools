<script setup>
// Generated at build time from the Cargo manifests and Cargo.lock (see cargo-deps.js).
import deps from "virtual:cargo-deps";

const crate = (name) => `https://crates.io/crates/${name}`;
// Most crates are dual-licensed ("MIT OR Apache-2.0"); show MIT and keep the full text on hover.
const shortLicense = (license) => (license?.includes("MIT") ? "MIT" : license ?? "—");

// Crates every tool uses come first, then the rest; each group alphabetical.
const usedByAll = (d) => d.usedBy.length === deps.crates.length;
const rows = [...deps.external.filter(usedByAll), ...deps.external.filter((d) => !usedByAll(d))];
</script>

<template>
  <h1>Dependencies</h1>
  <p class="lede">
    Every Rust crate the tools depend on directly, with the version in <code>Cargo.lock</code> and its license from crates.io.
  </p>
  <p class="muted">
    v{{ deps.version }} · {{ deps.external.length }} direct dependencies ·
    {{ deps.lockedPackages }} packages in the full lock file.
  </p>

  <div class="table">
    <table>
      <thead>
        <tr><th>Crate</th><th>Version</th><th>License</th><th>Used by</th></tr>
      </thead>
      <tbody>
        <tr
          v-for="(d, i) in rows"
          :key="d.name"
          :class="{ 'group-start': i > 0 && usedByAll(d) !== usedByAll(rows[i - 1]) }"
        >
          <td><a :href="crate(d.name)">{{ d.name }}</a></td>
          <td>{{ d.versions.join(", ") || "—" }}</td>
          <td :title="d.license ?? undefined">{{ shortLicense(d.license) }}</td>
          <td v-if="usedByAll(d)" class="muted">all</td>
          <td v-else>
            <span
              v-for="c in d.usedBy"
              :key="c"
              class="user"
              :class="{ muted: c === 'ty-common' }"
            >{{ c }}</span>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
