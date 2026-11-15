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
    Small desktop tools for Linux, written in Rust. Most sit in the system tray, and each
    has a command-line interface for scripts and keybindings. I use them daily on i3.
    They were built for X11 and also run on supported Wayland compositors.
    They're packaged with Nix and <a :href="license">MIT licensed</a>.
  </p>
  <p class="muted">v{{ deps.version }} · <RouterLink to="/deps">dependencies</RouterLink></p>

  <nav class="tray-bar" aria-label="Desktop bar preview: explore the tools">
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

  <h2>How they fit your desktop</h2>
  <ul class="plain">
    <li>There's no telemetry or separate account to create. Tools that use online services, like tyci, need those services' accounts and a connection.</li>
    <li>The apps are written in native Rust and don't use Electron.</li>
    <li>Each tool has a YAML config file you can keep in your dotfiles.</li>
    <li>One Nix flake provides reproducible packages and Home Manager modules.</li>
    <li>You can review the <RouterLink to="/deps">dependencies and their licenses</RouterLink>.</li>
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
    Wayland support depends on the protocols your compositor implements, including layer-shell,
    screencopy, data-control and virtual-keyboard. The tools support sway, Hyprland, river and
    similar compositors. KDE supports most of these protocols; GNOME doesn't offer them.
    Check each tool's page for its requirements and Wayland limitations. The Nix packages
    include the helpers each tool needs, such as wl-clipboard, wtype, grim and wf-recorder.
  </p>

  <h2>Install</h2>
  <p>Try a tool or install the whole collection:</p>
  <CodeBlock :code="tryOne" lang="shell" />
  <p>With Home Manager, import the modules and enable the tools you want:</p>
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
