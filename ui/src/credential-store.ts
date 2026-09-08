import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export type CredentialAvailability = "available" | "missing" | "unavailable";
export interface CredentialStatus { availability: CredentialAvailability; source?: "credential_store" | "environment" | "not_required" }
export interface SecretProtector {
  available(): boolean;
  encrypt(value: string): Buffer;
  decrypt(value: Buffer): string;
}
interface CredentialFile { version: 1; records: Record<string, { encrypted: string; updatedAt: string }> }

function secretId(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}:(generation|advisor|evaluator)$/i.test(value)) throw new Error("Invalid project credential identity.");
  return value.toLowerCase();
}
function secretValue(value: unknown): string {
  if (typeof value !== "string" || value.length < 8 || value.length > 8192 || /[\u0000-\u001f]/.test(value)) throw new Error("Enter a credential between 8 and 8192 characters without control characters.");
  return value;
}
function emptyFile(): CredentialFile { return { version: 1, records: {} }; }

/** Main-process-only encrypted credential references. Plaintext is never serialized. */
export class CredentialStore {
  constructor(readonly file: string, private protector: SecretProtector, private environment: NodeJS.ProcessEnv = process.env) {}

  private read(): CredentialFile {
    let source: string;
    try { source = readFileSync(this.file, "utf8"); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return emptyFile(); throw error; }
    try {
      const parsed = JSON.parse(source) as CredentialFile;
      if (parsed?.version !== 1 || !parsed.records || typeof parsed.records !== "object" || Array.isArray(parsed.records)) throw new Error("unsupported format");
      for (const [id, record] of Object.entries(parsed.records)) {
        secretId(id);
        if (!record || typeof record.encrypted !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(record.encrypted) || typeof record.updatedAt !== "string" || !Number.isFinite(Date.parse(record.updatedAt))) throw new Error("invalid encrypted record");
      }
      return parsed;
    } catch { throw new Error("The encrypted credential index is unreadable. It has not been changed."); }
  }

  private write(value: CredentialFile): void {
    mkdirSync(dirname(this.file), { recursive: true });
    const temporary = this.file + "." + randomUUID() + ".tmp";
    try {
      writeFileSync(temporary, JSON.stringify(value, null, 2) + "\n", { flag: "wx", mode: 0o600 });
      renameSync(temporary, this.file);
    } finally {
      try { unlinkSync(temporary); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
    }
  }

  set(id: unknown, value: unknown): void {
    const key = secretId(id), plaintext = secretValue(value);
    if (!this.protector.available()) throw new Error("Operating-system credential encryption is unavailable. Use the configured environment-variable fallback instead.");
    const file = this.read();
    file.records[key] = { encrypted: this.protector.encrypt(plaintext).toString("base64"), updatedAt: new Date().toISOString() };
    this.write(file);
  }

  remove(id: unknown): void {
    const key = secretId(id), file = this.read();
    if (!(key in file.records)) return;
    delete file.records[key]; this.write(file);
  }

  status(id: unknown, environmentFallback?: string): CredentialStatus {
    const key = secretId(id);
    if (environmentFallback && this.environment[environmentFallback]) return { availability: "available", source: "environment" };
    if (!this.protector.available()) return { availability: "unavailable" };
    let record: CredentialFile["records"][string] | undefined;
    try { record = this.read().records[key]; } catch { return { availability: "unavailable" }; }
    if (!record) return { availability: "missing" };
    try {
      return this.protector.decrypt(Buffer.from(record.encrypted, "base64")) ? { availability: "available", source: "credential_store" } : { availability: "unavailable" };
    } catch { return { availability: "unavailable" }; }
  }

  /** Used only by a main-process launcher to populate an allowlisted environment variable. */
  resolve(id: unknown, environmentFallback?: string): string | undefined {
    const key = secretId(id);
    if (environmentFallback && this.environment[environmentFallback]) return this.environment[environmentFallback];
    if (!this.protector.available()) return undefined;
    let record: CredentialFile["records"][string] | undefined;
    try { record = this.read().records[key]; } catch { return undefined; }
    if (!record) return undefined;
    try { return this.protector.decrypt(Buffer.from(record.encrypted, "base64")) || undefined; }
    catch { return undefined; }
  }
}
