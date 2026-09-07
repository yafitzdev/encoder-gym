# Encoder Gym desktop shell

A frameless Electron desktop app for the Encoder Gym platform.

> Status: **read-only control-plane presentation prototype**. Only the single
> active **Nomos repair** project is shown, using static evidence copied from
> the repository's documented optimization outcomes.

## Domain model (current)

- A **project** maps to a local folder and owns shared benchmark authority.
- A project contains immutable optimization **recipes/runs**.
- Each **recipe** carries:
  - dataset and model snapshots;
  - a bounded training policy;
  - persisted stage progression;
  - development and sealed evaluation evidence;
  - finite budget usage, a deterministic decision, and row-free provenance.

## Views

- **Sidebar** — local control-plane identity and Projects tree
  (project → its immutable runs).
- **Project page** — latest decision, stage progression, development gates,
  finite budgets, optimization history, and project authority.
- **Recipe page** — full run decision, evaluation evidence, run graph,
  provenance, budget ledger, and read-only operator commands.

The renderer remains intentionally read-only. Authoritative mutations stay in
the CLI until the persistence and command boundaries are designed explicitly.

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
