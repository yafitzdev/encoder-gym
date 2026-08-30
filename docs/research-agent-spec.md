# Authenticity research agent specification

## Objective

Add an independently useful, CLI-first research capability that studies public,
permitted sources and produces evidence-backed guidance for synthetic-data
prompt construction. The capability improves how closely generated input text
resembles the intended real-world channel. It does not copy sources into the
dataset and does not generate, train, evaluate, or optimize by itself.

The agent loop uses Pi. The application owns all policy, artifacts, tools,
budgets, and approval rules; Pi is a replaceable orchestration adapter.

## Boundary

```text
research brief + pinned dataset/semantic facts
  -> bounded Pi reasoning and tool loop
  -> application-owned search/fetch/evidence tools
  -> immutable evidence, claims, and authenticity profile
  -> explicit append-only review
  -> approved dataset binding
  -> resolved authenticity context
  -> existing generation prompt builder
```

`research-core` owns the provider-neutral domain and ports. It imports no Pi,
Node, SQLite, HTTP, CLI, prompt, or generation-backend types. The Pi adapter,
search/fetch adapters, SQLite implementation, and CLI are outside the core.

Semantic catalog profiles and authenticity profiles remain distinct. Semantic
profiles explain what a label or dimension value means. Authenticity profiles
describe observable input form: language, structure, noise, ambiguity, missing
context, channel artifacts, and synthetic-looking anti-patterns.

## Resolved brief

Before any external call, the application validates and persists a strict
versioned brief. It pins:

- dataset identity and fingerprint;
- task, labels, dimensions, and optional semantic-context identity;
- language and optional region, period, audience, channel, and domain;
- explicit research questions and source-diversity target;
- allowed/blocked domains and source classes;
- finite model-turn, search, page, byte, token, duration, retry, and cost limits;
- Pi provider/model identifiers and only the API-key environment-variable name;
- required authenticity-profile sections.

Defaults are resolved before persistence. The brief cannot contain raw
credentials. Hard local safety maxima reject accidentally unbounded briefs.

## Agent loop and tools

The Pi agent iteratively plans, searches, fetches, records evidence, inspects
gaps and contradictions, revises its plan, drafts the profile, and stops. It is
not a single synthesis prompt. Its only capabilities are narrow
application-owned tools equivalent to:

- `search_web`
- `fetch_page`
- `record_evidence`
- `inspect_evidence`
- `draft_profile`
- `finish_research`

The adapter receives no general shell, filesystem, database, process, or
unrestricted network tool. The host validates every tool request against the
persisted brief, cancellation state, and remaining budget. External content is
wrapped as untrusted source data and cannot modify the system prompt, tool
definitions, budget, source policy, or stop rules.

Every external intent is durable before I/O. Outcomes are `succeeded`, `failed`,
or `interrupted`; an interrupted call consumes the relevant attempt allowance
because its external outcome can be unknown. Retries are finite. Cancellation
prevents new calls and retains completed evidence.

## Evidence and synthesis

Evidence retains a canonical URL, title, originating query, source class,
retrieval time, content hash, small bounded excerpt, location where available,
derived observation, applicability, confidence, run identity, tool-call
identity, and fingerprint. Whole pages are not retained as dataset material.

Claims reference supporting and conflicting evidence. A claim without support
must explicitly declare that it is an inference. Profile sections reference
claims and separate observations from actionable generation instructions.
Profiles record source and run fingerprints, uncertainty, caveats, and their own
immutable fingerprint.

Basic exact/normalized comparison protects against reproducing retained
excerpts. Semantic/embedding similarity is not part of this capability.

## Review and handoff

Research ends in `awaiting_review`; it never starts generation. Reviews are
append-only `approve`, `reject`, or `request_revision` decisions with reviewer
and reason. Only the current explicit approval for the exact profile
fingerprint can create a binding.

A resolved authenticity context pins the dataset, binding, approval, profile
version, profile fingerprint, sections, instructions, caveats, and resolution
fingerprint. Generation copies it into an immutable job assignment and pins the
context fingerprint, prompt-template version, and normalized source-excerpt
guard fingerprint in the execution specification. Prompt construction receives
only abstract profile guidance: it never receives evidence excerpts or page
content. Validation receives the separately reconstructed excerpt guard and
rejects an exact normalized source match. Prompt preview exposes the same pinned
guidance. No search or Pi call occurs during generation, and generation without
a bound profile remains backward compatible.

## Durability

Research run states are `queued`, `running`, `awaiting_review`, `failed`, and
`cancelled`. The terminal stop reason distinguishes sufficient evidence, budget
exhaustion, cancellation, provider failure, validation failure, and
interruption. Progress and usage are derived from persisted calls and evidence.

Startup recovery interrupts orphaned calls and runs. It may resume from durable
evidence, but it cannot silently replay an uncertain paid call or reset a
budget. Profile revisions supersede rather than mutate previous artifacts.

## CLI contract

The CLI provides strict brief validation, start, status/watch, evidence/profile
inspection, cancellation, review, binding, and resolved-context commands. Human
and stable JSON output follow existing CLI conventions. Starting prints the
resolved external-call limits. No HTTP or graphical UI is added.

## Verification

Ordinary verification is offline. A scripted Pi model, fake search provider,
fake fetcher, and checked-in corpus cover iterative planning, evidence gaps,
conflicts, budget exhaustion, cancellation, retry/interruption recovery, source
policy, prompt-injection isolation, citation integrity, immutable review,
approval gating, prompt handoff, and the absence of research calls during
generation. Live provider testing is ignored, credential-gated, and explicitly
spend-bounded.

## Non-goals

This capability does not add ModernBERT, local label routing, web-source rows,
semantic deduplication, general crawling, multi-agent swarms, automatic
generation/training, UI/API expansion, cloud execution, authentication, or
access to sealed evaluation evidence.
