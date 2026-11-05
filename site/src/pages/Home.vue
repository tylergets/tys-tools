<script setup>
import { tools, license } from "../tools.js";
import deps from "virtual:cargo-deps";
import ToolIcon from "../ToolIcon.vue";
</script>

<template>
  <h1>ty's tools</h1>
  <p class="lede">
    Small desktop tools for Linux. Each one does one job, most sit in the system tray, and
    all of them can be scripted from the command line. Built for X11 and used daily on i3;
    they run on Wayland too. Written in Rust, packaged with Nix, <a :href="license">MIT licensed</a>.
  </p>
  <p class="muted">v{{ deps.version }} · <RouterLink to="/deps">dependencies</RouterLink></p>

  <h2>Why use them</h2>
  <ul class="plain">
    <li><strong>Private.</strong> No telemetry, no accounts, no cloud. Works offline.</li>
    <li><strong>Plain config.</strong> One YAML file per tool, ready for your dotfiles.</li>
    <li><strong>Scriptable.</strong> A CLI for everything, made for bars and keybindings.</li>
    <li><strong>Small and fast.</strong> Native Rust, no Electron.</li>
    <li><strong>Zero idle cost.</strong> No polling. When you're not using them, they sleep.</li>
    <li><strong>Quiet.</strong> No setup wizards, no pop-ups, no nagging.</li>
    <li><strong>No ads.</strong> No upsells, no "pro" version.</li>
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
  <pre><code># try one
nix run github:tylergets/tys-tools#tyshot

# install all of them
nix profile install github:tylergets/tys-tools</code></pre>
  <p>Home Manager, all tools at once:</p>
  <pre><code>inputs.tys-tools.url = "github:tylergets/tys-tools";

imports = [ inputs.tys-tools.homeManagerModules.default ];
programs.tyboard.enable = true;
programs.tyshot.enable = true;</code></pre>
  <p>From source:</p>
  <pre><code>nix develop
cargo run -p tyboard -- run</code></pre>
</template>
