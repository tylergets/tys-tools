<script setup>
import { computed, ref, watch } from "vue";
import captures from "../public/screenshots/manifest.json";

const { name } = defineProps({
  name: { type: String, required: true },
});
const views = computed(() => Object.values(captures).filter(capture => capture.tool === name));
const selected = ref(0);
watch(() => name, () => { selected.value = 0; });
const capture = computed(() => views.value[selected.value]);
const source = computed(() => `${import.meta.env.BASE_URL}screenshots/${capture.value.scene}.png?v=${capture.value.sha256.slice(0, 12)}`);
</script>

<template>
  <figure class="tool-screenshot">
    <div v-if="views.length > 1" class="views" aria-label="Screenshot views">
      <button v-for="(view, index) in views" :key="view.scene" type="button"
        :aria-pressed="selected === index" @click="selected = index">{{ view.caption }}</button>
    </div>
    <a :href="source" :aria-label="`Open full-size ${name} screenshot`">
      <img
        :src="source"
        :alt="`${name}: ${capture.caption}`"
        :width="capture.width"
        :height="capture.height"
        loading="lazy"
        decoding="async"
      />
    </a>
    <figcaption>
      {{ capture.caption }} · sample data ·
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
  max-width: 100%;
  height: auto;
}
.views {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 16px;
}
.views button {
  font: inherit;
  color: inherit;
  background: transparent;
  border: 1px solid currentColor;
  padding: 6px 10px;
  cursor: pointer;
}
.views button[aria-pressed="true"] {
  font-weight: 700;
  text-decoration: underline;
  text-underline-offset: 4px;
}
.tool-screenshot figcaption {
  margin-top: 8px;
  font-size: 12px;
  color: #999;
}
</style>
