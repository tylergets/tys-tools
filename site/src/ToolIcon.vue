<script setup>
import { computed } from "vue";

// The same SVGs the tray uses (crates/ty-common/icons). They draw in currentColor.
const icons = import.meta.glob("../../crates/ty-common/icons/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
});

const props = defineProps({ name: { type: String, required: true } });
const svg = computed(() => icons[`../../crates/ty-common/icons/${props.name}.svg`]);
</script>

<template>
  <span class="icon" aria-hidden="true" v-html="svg" />
</template>
