<script setup>
import { tools, license } from "../tools.js";
import deps from "virtual:cargo-deps";
import ToolIcon from "../ToolIcon.vue";
import CodeBlock from "../CodeBlock.vue";

const tryOne = `# try one
nix run github:tylergets/tys-tools#tyshot

# install all of them
nix profile install github:tylergets/tys-tools`;
const homeManager = `inputs.tys-tools.url = "github:tylergets/tys-tools";

imports = [ inputs.tys-tools.homeManagerModules.default ];
programs.tyboard.enable = true;
programs.tyshot.enable = true;`;
const fromSource = `nix develop
cargo run -p tyboard -- run`;
</script>

<template>
  <h1>ty's tools</h1>
  <p class="lede">
    Small desktop tools for Linux. Each one does one job, most sit in the system tray, and
    all of them can be scripted from the command line. Built for X11 and used daily on i3;
    they run on Wayland too. Written in Rust, packaged with Nix, <a :href="license">MIT licensed</a>.
  </p>
  <p class="muted">v{{ deps.version }} · <RouterLink to="/deps">dependencies</RouterLink></p>

  <nav class="tray-bar" aria-label="Desktop bar preview — explore the tools">
    <span class="tray-workspace" aria-hidden="true">1</span>
    <span class="tray-title" aria-hidden="true">ty's tools</span>
    <div class="tray-icons">
      <RouterLink
        v-for="t in tools"
        :key="t.name"
        :to="`/${t.name}`"
        :aria-label="`${t.name} — ${t.tagline}`"
        :title="`${t.name} — ${t.tagline}`"
      >
        <ToolIcon :name="t.name" />
      </RouterLink>
    </div>
    <span class="tray-clock" aria-hidden="true">Nov 11&nbsp; 11:28 PM</span>
  </nav>

  <h2>Why use them</h2>
  <ul class="plain">
    <li><strong>Private.</strong> No telemetry, no accounts, no cloud. Works offline.</li>
    <li><strong>Lightweight.</strong> Native Rust, no Electron. Idle tools sleep, not poll.</li>
    <li><strong>Scriptable.</strong> A CLI for everything, made for bars and keybindings.</li>
    <li><strong>Plain config.</strong> One YAML file per tool, ready for your dotfiles.</li>
    <li><strong>Reproducible.</strong> One Nix flake, Home Manager modules included.</li>
    <li><strong>Minimal, auditable dependencies.</strong> <RouterLink to="/deps">See the list</RouterLink>.</li>
    <li><strong>Open source.</strong> <a :href="license">MIT licensed</a>.</li>
  </ul>

  <h2>Tools</h2>
  <ul class="tools">
    <li v-for="t in tools" :key="t.name">
      <h3>
        <ToolIcon :name="t.name" />
        <RouterLink :to="`/${t.name}`">{{ t.name }}</RouterLink>
      </h3>
      <p class="tagline">{{ t.tagline }}</p>
      <p>{{ t.summary }}</p>
    </li>
  </ul>

  <h2>Wayland</h2>
  <p>
    Every tool runs on Wayland compositors with the wlroots protocols (layer-shell, screencopy,
    data-control, virtual-keyboard): sway, Hyprland, river and similar. KDE covers most of it;
    GNOME doesn't offer these protocols. Each tool's page says what changes on Wayland, and
    the Nix packages bring the helpers it needs (wl-clipboard, wtype, grim, wf-recorder).
  </p>

  <h2>Install</h2>
  <p>Everything lives in one Nix flake.</p>
  <CodeBlock :code="tryOne" lang="shell" />
  <p>Home Manager, all tools at once:</p>
  <CodeBlock :code="homeManager" lang="nix" />
  <p>From source:</p>
  <CodeBlock :code="fromSource" lang="shell" />
</template>

<style scoped>
.tray-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 4px 10px 4px 4px;
  margin: 24px 0 32px;
  background: #252525;
  color: #d4d4d4;
  border-bottom: 2px solid #777;
  font-size: 12px;
  line-height: 1;
}

.tray-workspace {
  align-self: stretch;
  display: grid;
  place-items: center;
  min-width: 28px;
  border-bottom: 2px solid #e5c453;
  background: #373737;
}

.tray-title {
  white-space: nowrap;
  margin-right: auto;
}

.tray-icons {
  display: flex;
  flex-wrap: wrap;
  gap: 2px;
}

.tray-icons a {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 32px;
  color: #d4d4d4;
  text-decoration: none;
}

.tray-icons a:nth-child(4n) {
  color: #e5c453;
}

.tray-icons a:nth-child(7n) {
  color: #e87979;
}

.tray-icons a:hover,
.tray-icons a:focus-visible {
  background: #454545;
  color: #fff;
  outline: 1px solid #e5c453;
  outline-offset: -1px;
}

.tray-icons .icon {
  width: 17px;
  height: 17px;
  margin: 0;
}

.tray-clock {
  white-space: nowrap;
}

@media (max-width: 680px) {
  .tray-title,
  .tray-clock {
    display: none;
  }

  .tray-icons {
    margin-left: auto;
  }
}
</style>
