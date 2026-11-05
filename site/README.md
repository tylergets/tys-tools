# site

The project website. Vue 3 + Vite + vue-router.

```
site/
  index.html            # Vite entry
  cargo-deps.js         # Vite plugin: reads Cargo.toml/Cargo.lock for the /deps page
  src/
    main.js
    App.vue             # header, footer
    router.js           # routes
    tools.js            # all tool content: cards, nav and tool pages
    pages/Home.vue
    pages/Tool.vue      # template for every /<tool> page
    pages/Deps.vue      # /deps: every crate's Rust dependencies and locked versions
    styles/global.css   # all styles
```

## Develop

```bash
npm install
npm run dev       # http://localhost:5173
npm run build     # outputs dist/
npm run preview   # serve dist/
```

## Adding a tool

Add an entry to `src/tools.js`. The card and `/<tool>` page appear automatically.

## Dependencies page

`/deps` is built from the workspace `Cargo.toml`, each `crates/*/Cargo.toml` and
`Cargo.lock` at build time (`import deps from "virtual:cargo-deps"`). Nothing is committed;
the Pages workflow also runs when those files change, so the page stays current. Licenses come from the crates.io API (one
request a second, cached in `node_modules/.cache`). The site
version on the home page comes from the same place.

## Adding docs

Add pages under `src/pages/` (for example `src/pages/tyboard/Docs.vue`) and routes like
`/tyboard/docs`. If docs grow large, VitePress (also Vue + Vite) can render Markdown.

## Deploy

Pushes to `main` that touch `site/` build and publish it to GitHub Pages at
<https://tys.tools> (`.github/workflows/pages.yml`). The custom domain is set in the repo's
Pages settings. The router uses HTML5 history, so the workflow copies `index.html` to
`404.html` to serve unknown paths.
