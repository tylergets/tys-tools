import { createRouter, createWebHistory } from "vue-router";
import Home from "./pages/Home.vue";
import Tool from "./pages/Tool.vue";
import { findTool } from "./tools.js";

const routes = [
  { path: "/", component: Home },
  {
    path: "/:name",
    component: Tool,
    props: true,
    beforeEnter: (to) => (findTool(to.params.name) ? true : "/"),
  },
  { path: "/:pathMatch(.*)*", redirect: "/" },
];

export const router = createRouter({
  history: createWebHistory(),
  routes,
  scrollBehavior: () => ({ top: 0 }),
});

router.afterEach((to) => {
  document.title = to.params.name ? `${to.params.name} — ty's tools` : "ty's tools";
});
