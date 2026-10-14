# Doppelganger documentation website

An independent static VitePress site. All developer pages live under `/docs`; no CLI/API runtime is required to host it. The existing repository had no frontend framework, so this folder establishes the documentation stack without changing the engine or backend.

## Develop and build

Use Node.js 22+ and npm, in this folder:

```bash
npm ci
npm run dev
```

Open the server's printed URL followed by `/docs/quickstart`.

```bash
npm run build
npm run check
npx playwright install chromium
npm test
```

Build output is `.vitepress/dist`. `check` validates the generated routes, links and anchors. Browser tests exercise all docs routes, clipboard copy, search, theme persistence, TOC, previous/next navigation and mobile layout. This project uses JavaScript/Vue SFCs; VitePress compiles them during build. No separate TypeScript or lint scripts are declared.

Dependencies and the lockfile are pinned. If your npm configuration blocks dependency install scripts, approve the trusted `esbuild` postinstall according to your npm policy before building. This was necessary on the development host.

## Host independently

### Vercel

Set the project Root Directory to `documentation`, Framework Preset to **Other**, Build Command to `npm run build`, and Output Directory to `.vitepress/dist`. The included `vercel.json` declares the same settings and clean URLs. Use Node.js 22 or later.

### Render

Create a **Static Site**, set Root Directory to `documentation`, Build Command to `npm ci && npm run build`, and Publish Directory to `.vitepress/dist`. The `render.yaml` in this folder records these settings. Do not create a Rust web service to host the docs.

Deploy the generated files unchanged, including HTML routes, assets and search chunks. No catch-all SPA rewrite is required. Routes are static files, not backend endpoints.

Neither configuration deploys the migration-job API/worker. The configured GitHub remote currently clones empty; publish the source before Git-based hosting. No deployment was performed for this task.

## Maintain the content

Edit `docs/*.md`; edit `.vitepress/navigation.mjs` for order/groups. The default theme supplies local search, copy buttons, heading anchors, TOC and previous/next navigation. `.vitepress/theme` customizes only typography, colors and breadcrumbs. Avoid duplicating milestone status text without checking current source and command behavior.

The preparation helper is a tutorial utility that reads checked-in artifacts from the parent repository. The website build does not use the helper or require the parent repository: copying this folder alone is sufficient to build the site.

From the repository root, verify CLI examples with:

```bash
python3 documentation/scripts/verify-cli.py --binary doppelganger
```

See VERIFICATION.md for executed checks and distribution limitations.
