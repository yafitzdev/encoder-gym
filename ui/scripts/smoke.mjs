import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";
import electron from "electron";

const profile = mkdtempSync(join(tmpdir(), "encoder-gym-smoke-profile-"));
for (const extra of [[], ["--smoke-restart"]]) {
  const child = spawn(electron, [".", "--smoke-test", ...extra], {
    stdio: "inherit", windowsHide: true,
    env: { ...process.env, ENCODER_GYM_SMOKE_PROFILE: profile },
  });
  const code = await new Promise((resolve, reject) => { child.on("error", reject); child.on("exit", code => resolve(code ?? 1)); });
  if (code !== 0) process.exit(code);
}
console.log("Both independent Electron launches passed using the same saved project collection.");
