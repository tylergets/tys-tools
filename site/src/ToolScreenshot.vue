<script setup>
import { computed } from "vue";
import captures from "../public/screenshots/manifest.json";

const { name } = defineProps({
  name: { type: String, required: true },
});
const views = computed(() => Object.values(captures).filter(capture => capture.tool === name));
const source = capture => `${import.meta.env.BASE_URL}screenshots/${capture.scene}.png?v=${capture.sha256.slice(0, 12)}`;
</script>

<template>
  <div class="screenshots" :class="{ single: views.length === 1 }">
  <figure v-for="capture in views" :key="capture.scene" class="tool-screenshot">
    <a class="image" :href="source(capture)" :aria-label="`Open full-size ${name}: ${capture.caption}`">
      <img
        :src="source(capture)"
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
  </div>
</template>

<style scoped>
.screenshots {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr));
  align-items: start;
  gap: 24px;
  margin: 24px 0 32px;
}
.tool-screenshot {
  min-width: 0;
  margin: 0;
}
.image {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 280px;
}
.single .image {
  height: auto;
  justify-content: flex-start;
}
.tool-screenshot img {
  display: block;
  max-width: 100%;
  height: auto;
  width: auto;
  max-height: 100%;
  object-fit: contain;
}
.single img {
  max-height: 480px;
}
.tool-screenshot figcaption {
  margin-top: 8px;
  font-size: 12px;
  color: #999;
}
</style>
