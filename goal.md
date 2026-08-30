# Goal — Agentic Authenticity Research and Generation Handoff

Add a bounded research-agent capability that studies real-world examples and
turns its evidence into an immutable authenticity profile for the existing
synthetic-data generation pipeline.

Use the Pi agent runtime from the official `badlogic/pi-mono` project for the
reasoning and tool-calling loop. Verify and pin the current maintained Pi
package and version before adding dependencies; do not assume an obsolete
package name or undocumented API.

This goal deliberately defers ModernBERT, encoder-based token reduction, and
all other local-model routing. The high-impact outcome is better synthetic data
through grounded research, not lower generation-token usage.

## Product outcome

The user journey should be:

```text
dataset definition + dimensions + semantic context + research brief
  -> explicit bounded research run
  -> Pi plans, searches, reads, compares, and records evidence
  -> evidence-backed authenticity profile draft
  -> user inspection and explicit approval
  -> immutable profile binding to the dataset/generation plan
  -> existing prompt construction consumes the pinned profile
  -> existing generation backend produces rows shaped by that profile
```

The research agent does not collect web pages as training rows. It researches
how authentic inputs look: their language, structure, noise, ambiguity,
context, channel artifacts, and other observable characteristics. Its output is
structured guidance that helps the LLM generate more realistic novel rows.

Research and generation remain separate operations. Completing research must
never automatically start generation, training, evaluation, or a workflow.

## Architectural boundary

Pi is orchestration infrastructure, not the owner of product business rules.

- The Rust core owns research briefs, budgets, evidence, claims, authenticity
  profiles, lifecycle rules, review decisions, bindings, fingerprints, and
  provenance.
- A narrow Pi adapter owns the iterative model/tool loop. Pi-specific types may
  not leak into Rust domain contracts, persistence, generation planning,
  prompting, or CLI presentation.
- Keep Pi integration in a small local TypeScript package/process if the
  maintained Pi runtime is TypeScript-only. Communicate through a strict,
  versioned protocol such as JSON Lines over standard input/output.
- Document this process boundary in an ADR. Keep the main product, database,
  job policy, and CLI in Rust.
- The existing generation backend contract remains unchanged. It receives a
  normalized generation request and never searches the web or manages research.
- Prompt construction receives a normalized, approved authenticity context.
  It must not read Pi state or provider-specific objects.
- Search and page retrieval sit behind replaceable application-owned ports.
  No search vendor, browser library, or Pi API may become a domain dependency.

Before committing to the process protocol, implement the smallest executable
spike needed to validate Pi model injection, tool definitions, event streaming,
cancellation, and deterministic testing. Keep only the chosen path and record
the decision; do not leave competing integration scaffolds.

## Research domain

Introduce focused domain concepts only where they earn their place. Likely
artifacts include:

- `ResearchBrief`: the versioned user intent and research constraints.
- `ResearchRun`: immutable input references plus durable lifecycle and budgets.
- `ResearchEvidence`: a sourced observation with retrieval provenance.
- `ResearchClaim`: a synthesis linked to supporting or conflicting evidence.
- `AuthenticityProfile`: structured generation guidance derived from claims.
- `ProfileReview`: append-only approve, reject, or request-revision decision.
- `ProfileBinding`: the exact approved profile pinned to a dataset or plan.

Names may change if the codebase already has better vocabulary. Keep these
artifacts separate from generated rows and from the semantic catalog. Semantic
definitions explain what labels and dimensions mean; authenticity research
explains how realistic inputs appear. Prompt construction may combine their
normalized resolved contexts without merging their ownership.

## Research brief

Define a strict, versioned brief that can express:

- dataset/task/schema identity and the relevant labels and dimensions;
- pinned semantic-catalog context where available;
- target language, region, time period, audience, channel, and domain;
- explicit research questions and desired source diversity;
- allowed and blocked domains or source classes;
- maximum model turns, searches, fetched pages, bytes, tokens, wall-clock time,
  and estimated or actual external spend;
- model/provider configuration by public identifier and environment-variable
  name only, never a credential value;
- stopping criteria and the required profile sections.

Validate the brief before any paid or external call. Resolve all defaults into
the persisted run so later inspection does not depend on changing config.

## Explicitly agentic behavior

The Pi loop must genuinely iterate rather than execute one large prompt:

1. Interpret the brief, schema, dimensions, and semantic context.
2. Produce a visible research plan and query-coverage plan.
3. Search and inspect multiple permitted sources through narrow tools.
4. Record evidence and source provenance as it is found.
5. Compare sources, identify contradictions, and locate evidence gaps.
6. Revise the plan and continue while useful and within finite budgets.
7. Stop when sufficiency criteria are met or a hard budget is exhausted.
8. Draft a structured authenticity profile with evidence-linked claims.
9. Validate citations, uncertainty, profile schema, and novelty safeguards.
10. Persist the draft for human review.

Expose only application-owned tools similar to:

```text
search_web(query, filters)
fetch_page(url)
record_evidence(source, observation, applicability)
inspect_evidence(filters)
draft_profile(profile)
finish_research(summary)
```

Do not grant the research agent a general shell, unrestricted filesystem,
database access, arbitrary process execution, or unconstrained network access.
Tool implementations enforce source policy, budgets, size limits, rate limits,
cancellation, and persistence regardless of what the model requests.

Treat every fetched page as untrusted data, never as instructions. Delimit it
from system/tool instructions, prevent page text from changing tool policy, and
test prompt-injection resistance at the trust boundary.

## Evidence and authenticity profile

Persist enough provenance to audit the research without copying entire sites:

- canonical URL, title, query, source type, retrieval time, and content hash;
- bounded excerpt or derived observation and its location where practical;
- relevance, applicability, confidence, and any conflicting evidence;
- retrieval/fetch status and relevant usage or policy metadata;
- the Pi run, tool call, model, prompt/protocol, and brief fingerprints that
  produced the observation or synthesis.

The profile should be machine-readable and human-inspectable. Support relevant
sections such as:

- typical length and structural patterns;
- vocabulary, jargon, register, syntax, and formatting;
- spelling, grammar, shorthand, noise, and incomplete-message patterns;
- tone, emotion, urgency, and implied user intent;
- missing context, irrelevant details, ambiguity, and class-boundary patterns;
- channel, device, temporal, geographic, and audience artifacts;
- supported distributions or ranges, with uncertainty where evidence is weak;
- patterns that make synthetic text look artificial and should be avoided;
- concrete generation instructions, caveats, and unresolved disagreements.

Each claim and instruction must reference its supporting evidence or be marked
explicitly as an inference. Preserve source diversity and uncertainty instead
of letting a single page become the definition of authenticity.

Research examples are evidence, not rows to copy. Store minimal necessary
excerpts, instruct generation to abstract rather than reproduce them, and add
basic exact/normalized overlap safeguards against retained excerpts where
practical. Do not add embedding or semantic deduplication in this goal.

## Durable execution and control

Use a local, single-runner job model with durable states such as:

```text
queued -> running -> awaiting_review -> approved
                   -> failed
                   -> cancelled
```

Represent revision requests and superseding profiles without mutating history.
Persist tool-call intent and outcome so progress, usage, failure, and budgets
come from stored facts rather than terminal state. Backend errors should be
recorded and retried only within explicit policy. Cancellation must prevent new
external calls and leave the evidence already gathered inspectable.

Support safe recovery after a process crash without silently replaying paid
calls or exceeding the original budgets. A terminal run must be deterministic
about why it stopped: sufficient evidence, user cancellation, hard limit,
provider failure, validation failure, or another explicit reason.

## Review and generation handoff

The user must be able to inspect the research plan, sources, evidence, claims,
profile, uncertainty, usage, and stop reason before approval.

- Reviews are append-only and retain reviewer intent and reason.
- Only an approved profile can be bound for generation.
- A binding pins profile ID, version, content fingerprint, and relevant source
  artifact fingerprints. Later profile revisions do not alter existing plans or
  jobs.
- Generation jobs pin the resolved authenticity profile alongside existing
  semantic context, construction recipe, prompt-template identity, and backend
  configuration.
- Prompt preview must show exactly which profile sections influence generation.
- Generation performs no live research and cannot observe mutable Pi state.
- Existing deterministic field construction, label ownership, validators,
  deduplication, provenance, and generation-backend boundaries remain intact.

## CLI-first surface

Add a coherent CLI flow with names aligned to existing conventions. It should
cover the equivalent of:

```text
synth research brief-validate <FILE>
synth research start <FILE>
synth research status <RUN_ID>
synth research watch <RUN_ID>
synth research evidence <RUN_ID>
synth research profile <RUN_ID>
synth research cancel <RUN_ID>
synth research review <PROFILE_ID> --approve|--reject --reason <TEXT>
synth research bind <DATASET_OR_PLAN_ID> <PROFILE_ID>
synth research context <DATASET_OR_PLAN_ID>
```

Provide useful human output plus stable JSON output where the existing CLI does
so. Starting a run must print resolved limits and whether external calls will
occur. Status must expose progress and budget consumption from persistence.

Do not add or expand a graphical UI or HTTP API in this goal.

## Testing and acceptance

Ordinary tests must be deterministic, offline, and free of paid calls. Provide
a scripted/fake Pi model loop, fake search provider, fake fetcher, and a small
checked-in source corpus. Test at least:

- brief parsing, validation, canonicalization, and fingerprints;
- Pi protocol normalization without Pi types leaking into Rust core crates;
- iterative planning and evidence-gap behavior with scripted model responses;
- source allow/block policy, rate and size limits, and duplicate evidence;
- every hard budget, cancellation, retries, interruption, and crash recovery;
- prompt-injection content remaining inert at the tool boundary;
- evidence-to-claim citations, conflicting sources, uncertainty, and profile
  schema validation;
- immutable review history and rejection/revision behavior;
- approval gating, profile binding, fingerprint pinning, and generation prompt
  integration;
- proof that generation makes no search calls and that research never inserts
  generated or source rows into datasets;
- migrations, doctor/integrity checks, CLI JSON contracts, and backward
  compatibility for generation without a research profile.

The Pi package must have its own pinned lockfile plus formatting, type-check,
unit-test, and build commands. Keep a real web/model smoke test explicit,
credential-gated, tightly budgeted, and excluded from ordinary repository
gates.

## Delivery sequence

Implement in coherent, reviewable stages:

1. Specification, trust boundaries, ADR, Rust domain contracts, and pure rules.
2. Offline research loop using scripted agent decisions and fake tools.
3. Minimal Pi adapter/process and versioned protocol, proven with fake providers.
4. Durable SQLite runs, calls, evidence, claims, profiles, reviews, and bindings.
5. Replaceable real search/fetch adapters with policy and budget enforcement.
6. Approval, binding, prompt-preview, and existing generation integration.
7. CLI workflow, offline process-level acceptance tests, doctor checks, and docs.

After each coherent stage, run and fix:

```text
cargo fmt-check
cargo check-all
cargo lint
cargo test-all
```

Also run the Pi adapter package's pinned formatter, type-checker, tests, and
build. Review crate/package dependency directions and commit each working stage
separately.

## Explicit non-goals

Do not implement:

- ModernBERT or any encoder as a token-saving or labeling router;
- direct ingestion of web content as training rows;
- automatic generation, training, evaluation, or workflow execution after
  research;
- multi-agent swarms, autonomous optimization, or open-ended background work;
- general browser automation, crawling infrastructure, or a search-engine
  product;
- semantic/embedding deduplication;
- graphical UI, HTTP API expansion, cloud/distributed execution,
  authentication, or multi-user collaboration;
- changes that expose sealed evaluation evidence to generation or research.

## Completion criterion

This goal is complete when a fresh local database can use a checked-in research
brief, scripted Pi agent, and fake web corpus to run the full offline research
cycle; persist an auditable evidence trail; produce, inspect, approve, and bind
an immutable authenticity profile; preview the exact resulting generation
context; generate deterministic fake-backend rows that pin the profile's
fingerprint; prove that no research occurs during generation; pass doctor and
all Rust and Pi package gates; and document the corresponding real-provider
workflow without requiring it for acceptance.
