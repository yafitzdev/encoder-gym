# Goal: Self-contained managed execution packages

Status: implemented and verified.
Owner: managed workspace and Nomos adapter integration.
Last updated: 20 September 2026.

## Outcome

Make a managed Encoder Gym project the complete local execution unit for its
active encoder workflow. After onboarding, training, evaluation, diagnosis,
optimization, reporting, and resume must not depend on the original Nomos
checkout or selected virtual-environment path.

## Required behavior

1. Preview a clean isolated Nomos source and compatible Python without writing.
2. Show the exact file count and disk bytes required for managed custody.
3. Copy the verified Nomos workspace and a portable Python runtime into a
   content-addressed package below the project.
4. Reject links, reparse points, path escapes, source mutation during copying,
   incomplete Python capabilities, external Python import paths, dirty Git
   state, remote Git configuration, and project-identity mismatches.
5. Record only project-relative runtime/executable paths and the package
   fingerprint in a new append-only scientific binding.
6. Preserve the contained scientific store and all historical external
   bindings during migration.
7. Resolve the managed package from the current project root in every CLI and
   desktop execution path, including prepare, start, resume, training,
   evaluation, qualification, registration, promotion, and readiness.
8. Let `workspace verify` rehash package inventories and reproduce Python,
   adapter, project, model, data, and store identities.
9. Keep status/report reads passive and avoid full package scans unless deep
   verification or execution requires them.
10. Update the desktop flow to describe a managed copy, show its storage cost,
    and invoke the same CLI contract.

## Compatibility

- Existing `external-isolated` bindings remain readable migration inputs.
- Historical runs and stores are never rewritten.
- Existing projects need no eager directory or database migration; the
  `runtimes/` tree appears atomically on the first managed bind.
- New managed bindings require a package identity and relative paths.

## Acceptance

- Domain tests reject incomplete or absolute managed bindings and reproduce
  package identities.
- Packaging tests reject transient/unsafe inputs and preserve required files.
- A CLI rebind test moves the original runtime away and proves readiness still
  succeeds through the managed package.
- Core, CLI, local persistence, and desktop type/unit/build checks pass.
- The real Nomos project is rebound, deeply verified, and can begin or resume a
  run with the original checkout and venv paths no longer consulted.

Acceptance completed on 20 September 2026. The real Nomos project is bound to
managed package `sha256:19a57cad5f75cff7381f046d500ef841641485717785ac0d99f5257a22996f41`;
deep inventory, contained Python, Nomos project, and readiness checks passed.
The existing active optimization remains resumable from its unchanged
scientific store.

## Out of scope

- Cloud execution, shared package caches, downloads, containers,
  authentication, distributed workers, and changing Nomos scientific policy.
- Rewriting old scientific evidence or deduplicating historical run artifacts.
