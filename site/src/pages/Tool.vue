<script setup>
import { computed, ref, watch } from "vue";
import { findTool, nixConfig, repo } from "../tools.js";
import ToolIcon from "../ToolIcon.vue";
import CodeBlock from "../CodeBlock.vue";

const props = defineProps({ name: { type: String, required: true } });
const tool = computed(() => findTool(props.name));
const configFormat = ref("nix");
watch(() => props.name, () => { configFormat.value = "nix"; });
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

    <template v-if="tool.storage">
      <h2>Storage and sync</h2>
      <p>{{ tool.storage }}</p>
    </template>

    <h2>{{ tool.configTool ? `${tool.configTool} integration` : "Config" }}</h2>
    <div class="config-switcher" role="group" aria-label="Configuration format">
      <button
        v-for="format in ['nix', 'yaml']"
        :key="format"
        type="button"
        :aria-pressed="configFormat === format"
        aria-controls="config-example"
        @click="configFormat = format"
      >{{ format === "nix" ? "Nix" : "YAML" }}</button>
    </div>
    <p v-if="configFormat === 'nix'">
      Home Manager configuration. Add <code>inputs</code> and <code>config</code> to your module arguments;
      the module writes the YAML config for you.
    </p>
    <p v-else>
      YAML, read from <code>./{{ tool.configTool ?? tool.name }}.yaml</code> or
      <code>~/.config/{{ tool.configTool ?? tool.name }}/config.yaml</code>.
    </p>
    <div id="config-example">
      <CodeBlock
        :code="configFormat === 'nix' ? nixConfig(tool) : tool.config"
        :lang="configFormat"
      />
    </div>

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
