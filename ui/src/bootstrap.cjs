// CJS entry that Electron can launch; the real main process is ESM.
if (process.argv.includes("--smoke-test")) {
  if (process.env.ENCODER_GYM_SMOKE_OUTPUT) {
    const { writeFileSync } = require("node:fs");
    writeFileSync(process.env.ENCODER_GYM_SMOKE_OUTPUT, "ENCODER_GYM_SMOKE_OK\n", { encoding: "utf8", flag: "wx" });
  }
}
void import("./main.js");
