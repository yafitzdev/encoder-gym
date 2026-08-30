#!/usr/bin/env node

import { createInterface } from "node:readline";

import type { InputMessage, OutputMessage } from "./protocol.js";
import { ProtocolSession } from "./sidecar.js";

function write(message: OutputMessage): void {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

const session = new ProtocolSession(write);
await session.ready();

const lines = createInterface({ input: process.stdin, crlfDelay: Number.POSITIVE_INFINITY });
lines.on("line", (line) => {
  try {
    const message = JSON.parse(line) as InputMessage;
    session.handle(message);
  } catch (error) {
    write({ type: "failed", message: error instanceof Error ? error.message : String(error) });
  }
});
lines.on("close", () => session.abort("protocol input closed"));
