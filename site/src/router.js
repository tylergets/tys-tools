import { createRouter, createWebHistory } from "vue-router";
import Home from "./pages/Home.vue";
import Tool from "./pages/Tool.vue";
import Deps from "./pages/Deps.vue";
import { findTool } from "./tools.js";

const routes = [
  { path: "/", component: Home },
  { path: "/deps", component: Deps },
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
  const page = to.params.name ?? (to.path === "/deps" ? "dependencies" : null);
  document.title = page ? `${page} — ty's tools` : "ty's tools";
});
