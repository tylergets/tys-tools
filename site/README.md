# site

The project website. Vue 3 + Vite + vue-router.

```
site/
  index.html            # Vite entry
  src/
    main.js
    App.vue             # header, nav, footer
    router.js           # routes
    tools.js            # all tool content: cards, nav and tool pages
    pages/Home.vue
    pages/Tool.vue      # template for every /<tool> page
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

Add an entry to `src/tools.js`. The card, nav link and `/<tool>` page appear automatically.

## Adding docs

Add pages under `src/pages/` (for example `src/pages/tyboard/Docs.vue`) and routes like
`/tyboard/docs`. If docs grow large, VitePress (also Vue + Vite) can render Markdown.

## Deploy

Publish `dist/` to any static host. The router uses HTML5 history, so the host must serve
`index.html` for unknown paths (Netlify/Cloudflare Pages: SPA fallback; GitHub Pages: copy
`index.html` to `404.html`).
