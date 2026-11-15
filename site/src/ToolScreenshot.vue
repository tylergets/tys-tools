<script setup>
import { computed } from "vue";

const { name, compact = false } = defineProps({
  name: { type: String, required: true },
  compact: Boolean,
});
const source = computed(() => `${import.meta.env.BASE_URL}screenshots/${name}.png`);
</script>

<template>
  <figure :class="['tool-screenshot', { compact }]">
    <a :href="source" :aria-label="`Open full-size ${name} screenshot`">
      <img
        :src="source"
        :alt="`${name} running in an isolated Linux desktop with sample data`"
        width="1200"
        height="800"
        loading="lazy"
        decoding="async"
      />
    </a>
    <figcaption v-if="!compact">
      Real app UI · sample data · Adwaita dark ·
      <a href="https://github.com/tylergets/tys-tools/tree/main/site/screenshots">Repeatable Xvfb capture</a>
    </figcaption>
  </figure>
</template>

<style scoped>
.tool-screenshot {
  margin: 24px 0 32px;
}
.tool-screenshot img {
  display: block;
  width: 100%;
  height: auto;
  border: 1px solid #454545;
  border-radius: 6px;
  background: #13161b;
}
.tool-screenshot figcaption {
  margin-top: 8px;
  font-size: 12px;
  color: #999;
}
.compact {
  margin: 16px 0 0;
}
</style>
