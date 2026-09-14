import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { CredentialStore } from "../dist/evidence/credential-store.js";

class Protector {
  enabled = true;
  available() { return this.enabled; }
  encrypt(value) { return Buffer.from([...Buffer.from(value)].map(byte => byte ^ 0x5a)); }
  decrypt(value) { return Buffer.from([...value].map(byte => byte ^ 0x5a)).toString(); }
}

test("project-role credentials are encrypted, isolated, replaceable, and never returned by status", () => {
  const root = mkdtempSync(join(tmpdir(), "gym-credentials-")), file = join(root, "credentials.json"), protector = new Protector();
  const store = new CredentialStore(file, protector, {}), project = randomUUID();
  const generation = `${project}:generation`, advisor = `${project}:advisor`;
  store.set(generation, "generation-secret-123"); store.set(advisor, "advisor-secret-456");
  const serialized = readFileSync(file, "utf8");
  assert.equal(serialized.includes("generation-secret-123"), false);
  assert.equal(serialized.includes("advisor-secret-456"), false);
  assert.deepEqual(store.status(generation), { availability: "available", source: "credential_store" });
  assert.deepEqual(store.status(advisor), { availability: "available", source: "credential_store" });
  assert.equal("value" in store.status(generation), false);
  store.set(generation, "replacement-secret-789");
  assert.equal(store.resolve(generation), "replacement-secret-789");
  assert.equal(store.resolve(advisor), "advisor-secret-456");
  store.remove(generation);
  assert.deepEqual(store.status(generation), { availability: "missing" });
});

test("environment fallback works without OS encryption and corrupt indexes fail without overwrite", () => {
  const root = mkdtempSync(join(tmpdir(), "gym-credential-fallback-")), file = join(root, "credentials.json"), protector = new Protector(), id = `${randomUUID()}:generation`;
  protector.enabled = false;
  const fallback = new CredentialStore(file, protector, { SYNTH_OPENAI_API_KEY: "environment-secret" });
  assert.deepEqual(fallback.status(id, "SYNTH_OPENAI_API_KEY"), { availability: "available", source: "environment" });
  assert.equal(fallback.resolve(id, "SYNTH_OPENAI_API_KEY"), "environment-secret");
  assert.deepEqual(fallback.status(id), { availability: "unavailable" });
  assert.throws(() => fallback.set(id, "another-secret"), /encryption is unavailable/);
  writeFileSync(file, "not-json"); protector.enabled = true;
  const before = readFileSync(file, "utf8");
  assert.throws(() => fallback.set(id, "another-secret"), /index is unreadable/);
  assert.equal(readFileSync(file, "utf8"), before);
});

test("credential identifiers and secret values are strictly bounded", () => {
  const store = new CredentialStore(join(mkdtempSync(join(tmpdir(), "gym-credential-invalid-")), "credentials.json"), new Protector(), {});
  store.set(`${randomUUID()}:connection:${randomUUID()}`, "connection-secret");
  assert.throws(() => store.set("global:generation", "valid-secret"), /identity/);
  assert.throws(() => store.set(`${randomUUID()}:generation`, "short"), /between 8 and 8192/);
  assert.throws(() => store.set(`${randomUUID()}:generation`, "line\nbreak-secret"), /control characters/);
});
