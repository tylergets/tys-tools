<script setup>
import { computed } from "vue";
import { findTool, repo } from "../tools.js";
import ToolIcon from "../ToolIcon.vue";
import CodeBlock from "../CodeBlock.vue";

const props = defineProps({ name: { type: String, required: true } });
const tool = computed(() => findTool(props.name));
</script>

<template>
  <template v-if="tool">
    <h1><ToolIcon :name="tool.name" />{{ tool.name }}</h1>
    <p class="lede">{{ tool.tagline }}</p>
    <p>{{ tool.summary }}</p>

    <h2>Features</h2>
    <ul class="plain">
      <li v-for="f in tool.features" :key="f">{{ f }}</li>
    </ul>

    <h2>Requirements</h2>
    <ul class="plain">
      <li v-for="r in tool.requirements" :key="r">{{ r }}</li>
    </ul>
    <p class="muted">The Nix package puts runtime tools on <code>PATH</code> for you.</p>

    <template v-if="tool.wayland">
      <h2>On Wayland</h2>
      <p>{{ tool.wayland }}</p>
    </template>

    <h2>Usage</h2>
    <CodeBlock :code="tool.usage" lang="shell" />

    <h2>Config</h2>
    <p>
      YAML, read from <code>./{{ tool.name }}.yaml</code> or
      <code>~/.config/{{ tool.name }}/config.yaml</code>.
    </p>
    <CodeBlock :code="tool.config" lang="yaml" />

    <h2>Install</h2>
    <CodeBlock :code="`nix run github:tylergets/tys-tools#${tool.name}`" lang="shell" />
    <p>Home Manager:</p>
    <CodeBlock
      :code="`imports = [ inputs.tys-tools.homeManagerModules.${tool.name} ];\nprograms.${tool.name}.enable = true;`"
      lang="nix"
    />

    <p>
      <a :href="`${repo}/tree/main/crates/${tool.name}`">Full README</a>
    </p>
  </template>
</template>
