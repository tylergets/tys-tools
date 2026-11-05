<script setup>
// Generated at build time from the Cargo manifests and Cargo.lock (see cargo-deps.js).
import deps from "virtual:cargo-deps";
import ToolIcon from "../ToolIcon.vue";
import { tools } from "../tools.js";

const crate = (name) => `https://crates.io/crates/${name}`;
const isTool = (name) => tools.some((t) => t.name === name);
const common = deps.crates.find((c) => c.name === "ty-common");
const others = deps.crates.filter((c) => c.name !== "ty-common");
const external = (c) => c.deps.filter((d) => !d.path);
const usesCommon = (c) => c.deps.some((d) => d.name === "ty-common");

const notes = (d) => {
  const n = [];
  if (d.kind !== "dependencies") n.push(d.kind.replace("-dependencies", ""));
  if (d.target) n.push(d.target);
  if (d.optional) n.push("optional");
  if (!d.defaultFeatures) n.push("no default features");
  return n.join(", ");
};
</script>

<template>
  <h1>Dependencies</h1>
  <p class="lede">
    Every Rust crate the tools depend on directly, with the version in <code>Cargo.lock</code>.
  </p>
  <p class="muted">
    v{{ deps.version }} · {{ deps.external.length }} direct dependencies ·
    {{ deps.lockedPackages }} packages in the full lock file. Updated on every site deploy.
  </p>

  <h2>All</h2>
  <div class="table">
    <table>
      <thead>
        <tr><th>Crate</th><th>Version</th><th>Required</th><th>Used by</th></tr>
      </thead>
      <tbody>
        <tr v-for="d in deps.external" :key="d.name">
          <td><a :href="crate(d.name)">{{ d.name }}</a></td>
          <td>{{ d.versions.join(", ") || "—" }}</td>
          <td class="muted">{{ d.req ?? "—" }}</td>
          <td>{{ d.usedBy.length === deps.crates.length ? "all" : d.usedBy.join(" ") }}</td>
        </tr>
      </tbody>
    </table>
  </div>

  <template v-for="c in [common, ...others].filter(Boolean)" :key="c.name">
    <h2 :id="c.name">
      <ToolIcon v-if="isTool(c.name)" :name="c.name" />
      <RouterLink v-if="isTool(c.name)" :to="`/${c.name}`">{{ c.name }}</RouterLink>
      <template v-else>{{ c.name }}</template>
    </h2>
    <p v-if="c.name === 'ty-common'" class="muted">Shared by every tool.</p>
    <p v-else-if="usesCommon(c)" class="muted">Plus <a href="#ty-common">ty-common</a>.</p>
    <div class="table">
      <table>
        <thead>
          <tr><th>Crate</th><th>Version</th><th>Features</th><th>Notes</th></tr>
        </thead>
        <tbody>
          <tr v-for="d in external(c)" :key="d.name + d.kind + d.target">
            <td><a :href="crate(d.name)">{{ d.name }}</a></td>
            <td>{{ d.versions.join(", ") || d.req }}</td>
            <td class="muted">{{ d.features.join(" ") }}</td>
            <td class="muted">{{ notes(d) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </template>
</template>
