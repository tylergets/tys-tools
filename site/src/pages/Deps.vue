<script setup>
// Generated at build time from the Cargo manifests and Cargo.lock (see cargo-deps.js).
import deps from "virtual:cargo-deps";

const crate = (name) => `https://crates.io/crates/${name}`;
</script>

<template>
  <h1>Dependencies</h1>
  <p class="lede">
    Every Rust crate the tools depend on directly, with the version in <code>Cargo.lock</code> and its license from crates.io.
  </p>
  <p class="muted">
    v{{ deps.version }} · {{ deps.external.length }} direct dependencies ·
    {{ deps.lockedPackages }} packages in the full lock file. Updated on every site deploy.
  </p>

  <div class="table">
    <table>
      <thead>
        <tr><th>Crate</th><th>Version</th><th>Required</th><th>License</th><th>Used by</th></tr>
      </thead>
      <tbody>
        <tr v-for="d in deps.external" :key="d.name">
          <td><a :href="crate(d.name)">{{ d.name }}</a></td>
          <td>{{ d.versions.join(", ") || "—" }}</td>
          <td class="muted">{{ d.req ?? "—" }}</td>
          <td>{{ d.license ?? "—" }}</td>
          <td>{{ d.usedBy.length === deps.crates.length ? "all" : d.usedBy.join(" ") }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
