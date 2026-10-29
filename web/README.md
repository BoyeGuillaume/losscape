# losscape web UI

Svelte 5 + SvelteKit 3, built as a static single-page app (`adapter-static`, fallback
`index.html`) that the Rust server serves from `web/build`.

- Components: [shadcn-svelte](https://shadcn-svelte.com) (`src/lib/components/ui`, generated,
  not hand-edited) with Tailwind 4.
- Charts: [LayerChart](https://layerchart.com) on canvas (`src/lib/components/app/MetricChart.svelte`).
- Metric data comes as tiles (`src/lib/charts/tiles.svelte.ts`, see docs/REQUIREMENTS.md
  Design 1); live points arrive over SSE (`src/lib/api/live.ts`).

```sh
npm install
npm run dev      # http://localhost:5173, proxies /api to the server on :8080
npm run check    # svelte-check
npm run lint     # prettier + eslint
npm run build    # -> build/, served by losscape-server
```

Needs Node 24 (`.nvmrc`).
