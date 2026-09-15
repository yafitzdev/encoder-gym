# Governed workflow generation supervision

## Objective

Make the Generation Quality Supervisor an optional execution mode of the
finite encoder workflow without creating a second generation, curation, or
snapshot pipeline. Operators choose understandable quality outcomes and
separate provider profiles. The application compiles those choices into the
existing detailed immutable supervisor contract once the exact generation plan
and current coverage exist.

## Lifecycle

The workflow retains its existing generation stage names. Their implementation
depends only on the immutable definition:

```text
initial allocation -> generation [supervised]
  -> completed supervisor qualification handoff
  -> curation review (awaiting user)
  -> curation approval -> snapshot -> training firewall -> training

proposal application -> dataset-diff generation [supervised]
  -> completed supervisor qualification handoff
  -> iteration curation review (awaiting user)
  -> curation approval -> iteration snapshot -> training firewall -> training
```

The definition may instead omit supervision, preserving the ordinary legacy
paths. `generation_supervision` and `quality_gate` are mutually exclusive:
supervision already performs full-population assessment and locally replays its
direct evidence into Dataset Qualification.

## Operator controls and deterministic compilation

The strict workflow manifest exposes:

- `quality_level`: economical, balanced, or strict;
- authenticity and diversity importance: off, normal, or high;
- maximum prompt repairs;
- desired monitoring segment size;
- finite generated-row, evaluator-token, Pi-token, duration, retry, and
  optional cost ceilings;
- manual or bounded pre-authorized prompt repair; and
- separate generator and evaluator profiles.

The compiler resolves every threshold, monitoring window, batch limit, prompt
protection rule, approval envelope, and execution budget. Presets are never
interpreted by a running supervisor. When a plan has fewer rows in a scope than
the desired window, contract construction deterministically clamps the exact
window and minimum evidence to the attainable positive count; it never silently
drops a scope or infers quality for an unassessed row.

Generator and evaluator profiles contain only non-secret configuration. Each
has its own backend, model, endpoint when external, protocol/configuration
parameters, fingerprint, and optional uppercase API-key environment-variable
name. A profile never contains a credential. Runtime adapters must reproduce
the pinned profile and resulting contract identities before provider work.

## Durable workflow boundary

Before starting or continuing supervised generation, the running workflow
attempt appends a `generation_supervisor_run` child-execution link. Contract and
run identities are deterministic for the workflow run, iteration, stage, and
plan, while their contents retain normal immutable fingerprints. Recovery
reuses the same supervisor run and invokes its existing recovery rules.
Cancellation closes the workflow authority first and then sets the linked
supervisor cancellation flag.

The stage advances the supervisor only to its next boundary. A deterministic
pause, diagnosis, revision review, or pending canary finishes the workflow
attempt as `awaiting_user` and links the exact contract, run, prompt/window,
and decision evidence. Repeated `workflow resume` calls are no-ops until an
authorized supervisor action changes that persisted boundary. A later resume
starts a new append-only workflow attempt for the same stage and same supervisor
run; it does not reset usage or create a replacement contract.

When the supervisor completes, the existing qualification finalizer creates or
reuses its immutable handoff, local replay audit/report, qualification
application, and ordinary curation proposal. The generation stage links all of
them using the artifact kinds already consumed by curation review. It does not
approve the proposal. The existing curation review, approved-manifest snapshot
application, training qualification assertion, and benchmark firewall remain
the only path forward.

## Usage and governance

Supervisor generated rows and child requests are reconciled into the parent
workflow's cumulative accepted-row, generation-attempt, and request budgets.
The detailed supervisor contract independently enforces evaluator, Pi, repair,
canary, duration, retry, token, and optional cost ceilings. Neither authority
can expand the other.

Manual repair mode always requires an append-only exact proposal review.
Pre-authorized mode permits only a change fitting its persisted revision kind,
scope, instruction, character, expiry, and remaining-budget envelope. Canary
evidence still decides activation. Curation approval is always manual in this
phase.

## Dependency and compatibility rules

`generation-supervisor-core` owns the outcome-control compiler and the resolved
provider-neutral supervision blueprint. `workflow-core` may pin that narrow
blueprint and decide the legal next stage. It owns no thresholds, prompt repair,
or quality logic. The CLI application assembles existing generation,
qualification, supervisor, and SQLite ports.

New optional definition fields use serde defaults so historical definitions
remain readable. Definitions omitting supervision reproduce their previous
fingerprints. No HTTP or graphical UI surface changes in this phase.

## Verification

Deterministic tests cover all preset mappings, provider-profile validation,
small-scope window resolution, mutual exclusion with the legacy quality gate,
both initial and iteration stage graphs, exact child linking, pause/resume
idempotency, cancellation/recovery, finalization into ordinary curation,
explicit manifest approval, qualified snapshot membership, training firewall,
provenance, Doctor, legacy definition loading, and a fresh-database offline CLI
flow. Ordinary verification performs no network call and uses no credential,
model download, GPU, or paid service.
