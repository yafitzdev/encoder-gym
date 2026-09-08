import { build } from "esbuild";
import { cpSync, mkdirSync, readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);

for (const module of ["projects", "project-registry", "managed-backend", "presentation-errors"]) await build({
  entryPoints: [fileURLToPath(new URL(`../src/${module}.ts`, import.meta.url))],
  outfile: fileURLToPath(new URL(`../dist/evidence/${module}.js`, import.meta.url)),
  bundle: true, platform: "node", format: "esm", logLevel: "info",
});

await build({
  entryPoints: [fileURLToPath(new URL("../src/evidence/read-workspace.ts", import.meta.url))],
  outfile: fileURLToPath(new URL("../dist/evidence/read-workspace.js", import.meta.url)),
  bundle: true, platform: "node", format: "esm", logLevel: "info",
});
for (const [source, name] of [["catalog", "catalog"], ["state", "navigation"]]) await build({
  entryPoints: [fileURLToPath(new URL(`../src/renderer/${source}.ts`, import.meta.url))],
  outfile: fileURLToPath(new URL(`../dist/evidence/${name}.js`, import.meta.url)),
  bundle: true, platform: "node", format: "esm", logLevel: "info",
});

// Electron main process -> ESM bundle (electron stays external; bootstrap.cjs imports it).
await build({
  entryPoints: [fileURLToPath(new URL("../src/main.ts", import.meta.url))],
  outfile: fileURLToPath(new URL("../dist/main.js", import.meta.url)),
  bundle: true,
  platform: "node",
  format: "esm",
  external: ["electron"],
  sourcemap: true,
  logLevel: "info",
});

// CJS entry Electron actually launches.
cpSync(fileURLToPath(new URL("../src/bootstrap.cjs", import.meta.url)), fileURLToPath(new URL("../dist/bootstrap.cjs", import.meta.url)));

// Preload -> CommonJS bundle (sandboxed preload, electron stays external).
await build({
  entryPoints: [fileURLToPath(new URL("../src/preload.ts", import.meta.url))],
  outfile: fileURLToPath(new URL("../dist/preload.cjs", import.meta.url)),
  bundle: true,
  platform: "node",
  format: "cjs",
  external: ["electron"],
  sourcemap: true,
  logLevel: "info",
});

// Renderer -> ESM bundle for the file:// page.
await build({
  entryPoints: [fileURLToPath(new URL("../src/renderer/app.ts", import.meta.url))],
  outfile: fileURLToPath(new URL("../dist/renderer/app.js", import.meta.url)),
  bundle: true,
  platform: "browser",
  format: "esm",
  sourcemap: true,
  logLevel: "info",
});

// Static renderer assets.
mkdirSync(fileURLToPath(new URL("../dist/renderer/", import.meta.url)), { recursive: true });
cpSync(fileURLToPath(new URL("../src/renderer/index.html", import.meta.url)), fileURLToPath(new URL("../dist/renderer/index.html", import.meta.url)));
cpSync(fileURLToPath(new URL("../src/renderer/styles.css", import.meta.url)), fileURLToPath(new URL("../dist/renderer/styles.css", import.meta.url)));

// Fail loudly if the shell stylesheet references a file that was not copied.
const shellStyles = readFileSync(fileURLToPath(new URL("../dist/renderer/styles.css", import.meta.url)), "utf8");
for (const match of shellStyles.matchAll(/@import\s+"([^"]+)"/g)) {
  const target = new URL(match[1], new URL("../dist/renderer/styles.css", import.meta.url));
  if (!existsSync(fileURLToPath(target))) {
    throw new Error("styles.css imports " + match[1] + ", but it was not copied into dist");
  }
}
console.log("Encoder Gym UI build complete.");
