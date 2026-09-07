# Encoder Gym desktop shell

A frameless Electron desktop app for the Encoder Gym platform.

> Status: **minimal skeleton + project/recipe model**. Only the single active
> **Nomos repair** project is shown. The app is shaped around a deliberate
> domain model being refined with the user.

## Domain model (current)

- A **project** maps to a local folder and owns a shared **evaluation suite** (D).
- A project contains **recipes** that compete against each other.
- Each **recipe** carries:
  - **A) dataset snapshots**
  - **B) instructions / configs**
  - **C) model snapshots**

## Views

- **Sidebar** — brand, inert **New project** placeholder, Projects tree
  (project → its recipes).
- **Project page** (click project) — task/folder/evaluation-suite facts,
  recipe list, and an *Evaluation results* card (deferred).
- **Recipe page** (click a recipe) — A/B/C buckets + commands.

No Overview, All runs, Workflows, or New run — removed pending product
decisions.

## What is kept

- `src/main.ts` — frameless main process; window-action IPC; hardened
  `BrowserWindow` (context isolation, sandbox, no node integration).
- `src/preload.ts` — typed bridge exposed as `window.encoderGym`.
- `src/renderer/**` — static `index.html` chrome, theme CSS, vanilla-TS
  renderer modules (`data.ts`, `state.ts`, `views.ts`, `app.ts`,
  `dom.ts`).
- `scripts/build.mjs` — esbuild bundles for main/preload/renderer and copies
  static assets into `dist/`.

## Development

```powershell
npm install
npm start          # build then launch Electron
npm run typecheck  # TypeScript only
npm run check      # typecheck + build
```

Mock domain data lives in `src/renderer/data.ts` and is kept separate from the
view modules so real persisted facts can replace it later.
