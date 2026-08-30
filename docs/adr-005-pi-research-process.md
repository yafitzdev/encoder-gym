# ADR 005: Isolate Pi behind a host-governed JSON Lines process

## Status

Accepted.

## Context

Authenticity research must use Pi for a genuine iterative reasoning and
tool-calling loop while the product remains a Rust application. Pi is a
TypeScript runtime. Importing Pi shapes into Rust domain contracts would couple
research policy and generation to a provider framework. Running the Pi coding
agent directly would also expose general coding tools that this research role
must never receive.

The maintained package was verified on 2026-08-30 as
`@earendil-works/pi-agent-core` 0.84.4, with Node 22.19 or newer. The repository
pins that exact version and its transitive dependency graph in the adapter
lockfile.

An executable spike proved model injection, a scripted faux provider, six
custom tools, sequential tool execution, lifecycle event streaming, a hard turn
ceiling, abort support, and deterministic tests through Pi's public `Agent`
API.

## Decision

Keep Pi in `adapters/research-agent-pi`, a small private Node package. Rust
starts its compiled executable as a local child process. The two sides exchange
one versioned JSON object per line over standard input and output.

Pi receives only these tools:

- `search_web`
- `fetch_page`
- `record_evidence`
- `inspect_evidence`
- `draft_profile`
- `finish_research`

Tool implementations stay in the Rust host. A Pi tool call becomes a protocol
request and remains pending until Rust returns a result. Rust therefore checks
the persisted brief, source policy, remaining budget, cancellation, and durable
call ledger before any external action. The adapter has no shell, filesystem,
database, process, or general browser tool.

The sidecar reports normalized lifecycle and usage events. It accepts only a
resolved, non-secret run specification and an API-key environment-variable
name; it never transmits the key. One sidecar handles one active run at a time.
Closing the protocol or cancelling the run aborts Pi and all pending tools.

Pi's faux provider drives ordinary offline tests, so the real Pi agent loop is
tested without network access or paid calls. Production models resolve through
Pi's maintained built-in model registry.

## Consequences

- Rust domain, SQLite, generation, and CLI code contain no Pi types.
- Every powerful action remains observable and enforceable by the application.
- The process protocol must be versioned and tested at both ends.
- Node is a deliberate local runtime prerequisite for agentic research only;
  the rest of the platform remains usable without it.
- Updating Pi requires an explicit dependency change, protocol smoke test, and
  adapter gate run.
