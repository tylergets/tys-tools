<script setup>
import { computed } from "vue";
import { highlight } from "./highlight.js";

const props = defineProps({
  code: { type: String, required: true },
  lang: { type: String, default: "" },
});
const html = computed(() => highlight(props.code, props.lang));
</script>

<template>
  <pre :class="['code', lang && `lang-${lang}`]"><code v-html="html"></code></pre>
</template>
