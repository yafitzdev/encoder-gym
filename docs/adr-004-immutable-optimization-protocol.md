# ADR 004 — Immutable optimization protocols

## Status

Accepted.

## Context

The original Slice 6 proposal command accepted only a budget and minimum
support. Its deterministic error-count allocation was useful, but the decision
policy and its safety constraints were implicit. Slice 6.1 needs multiple
scoring policies, constrained allocation, alternative scenarios, approvals,
and stale-evidence checks without allowing mutable project defaults to change a
proposal after construction starts.

Historical proposals already exist as immutable JSON payloads and must remain
decodable.

## Decision

`optimization-core` owns a strict `OptimizationProtocol`. It contains the
finite budget, evidence threshold, recommendation kinds, score and uncertainty
policies, deterministic tie rule and seed, eligibility filters, allocation
bounds, concentration limit, allocation increment, review exclusions,
comparison behavior, bounded training-candidate request, and explicit
infeasibility behavior.

Protocols are normalized before use. Set-like vectors are trimmed, sorted, and
required to be unique. Contradictory filters, non-finite numerical settings,
impossible minimums, and incompatible feature switches are rejected before
evidence is loaded. Fingerprints are calculated only for validated canonical
protocols.

New proposal payloads retain the complete resolved protocol and its SHA-256
fingerprint. The compatibility constructor resolves the historical budget and
minimum-support arguments into an explicit legacy protocol whose policy is
error-count scoring with unit allocation increments. Historical proposal JSON
deserializes with no protocol and retains its historical proposal fingerprint.

The protocol contains project-owned domain values only. SQLite rows, CLI
arguments, provider settings, trainer internals, and presentation types do not
cross this boundary.

## Consequences

- Later score, allocation, review, rebase, and campaign decisions can cite one
  immutable policy identity.
- Protocol files can become a CLI input without moving validation into the CLI.
- Exact legacy behavior remains available and is distinguishable from Slice
  6.1 proposals.
- Persistence will require an append-only migration to normalize protocol and
  proposal relationships used for filtering and integrity checks.
- Training-candidate configuration spaces remain a separate, typed source
  identity; this protocol only states whether a finite candidate set is
  requested and its maximum size.
