import type { ProviderConfiguration } from "./managed-workspace.js";
import type { CredentialStore, CredentialStatus } from "./credential-store.js";
import type { ProviderConnectionStore } from "./provider-connections.js";

const identity = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

/** Validate project scope before a launcher can ask the OS store for a key. */
export function providerCredentialBinding(projectId: string, provider: ProviderConfiguration): { id: string; environment?: string; connectionId?: string } {
  const reference = provider.secret, prefix = `${projectId}:connection:`;
  const connectionId = reference?.id.startsWith(prefix) ? reference.id.slice(prefix.length) : undefined;
  if (identity.test(projectId) && connectionId && identity.test(connectionId) && reference && reference.environmentFallback === undefined) {
    return { id: reference.id, connectionId, environment: `ENCODER_GYM_CONNECTION_${connectionId.replaceAll("-", "").toUpperCase()}_API_KEY` };
  }
  const expected = provider.role === "generation" ? "SYNTH_OPENAI_API_KEY" : provider.role === "advisor" ? "SYNTH_ADVISOR_API_KEY" : "SYNTH_EVALUATOR_API_KEY";
  if (identity.test(projectId) && reference?.id === `${projectId}:${provider.role}` && reference.environmentFallback === expected) return { id: reference.id, environment: expected };
  throw new Error(`The ${provider.role} credential reference is not safe for desktop execution. Reconfigure project providers.`);
}

/** Resolves exact identities only. A role assignment is never a secret alias. */
export class ProjectCredentials {
  constructor(private credentials: CredentialStore, private connections: ProviderConnectionStore) {}

  private connectionExists(id: string): boolean {
    const parts = id.split(":");
    const project = parts[0], connectionId = parts[2];
    return parts.length === 3 && parts[1] === "connection" && !!project && !!connectionId && identity.test(project) && identity.test(connectionId)
      && this.connections.get(project).connections.some(connection => connection.id === connectionId);
  }

  resolve(id: string, environmentFallback?: string): string | undefined {
    if (id.includes(":connection:")) return this.connectionExists(id) ? this.credentials.resolve(id) : undefined;
    return this.credentials.resolve(id, environmentFallback);
  }

  status(id: string, environmentFallback?: string): CredentialStatus {
    if (id.includes(":connection:")) return this.connectionExists(id) ? this.credentials.status(id) : { availability: "missing" };
    return this.credentials.status(id, environmentFallback);
  }
}
