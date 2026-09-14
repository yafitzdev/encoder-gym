# Input-first agentic optimization

Implementation status: settings contracts, strict CLI preview/authorization and
desktop history compatibility are implemented. The iterative executor and
iteration-aware GUI remain outstanding. The fixed-recipe executor explicitly
rejects agentic settings; it must not masquerade as an agent run.

The read-only CLI supports `workspace optimization-launch <PROJECT> preview
--setup <ID> --quick-test` or `--settings-file <STRICT_JSON>`. Omitting both
preserves the legacy preview byte shape. Authorization persists the exact
settings and their derived bounds; legacy `optimization-run start` rejects
those authorities before reserving a run until an executor can honor them.

This specification extends the fixed-recipe execution in
`optimization-launch-spec.md`. It is explicitly authorized by the September 14,
2026 product request. It permits a bounded agent and concurrent generation
requests inside one local run, not multiple trainers or distributed workers.

## Run settings

Setup retains the five ordinary input selectors. An Advanced disclosure owns
an optional user objective, maximum optimization iterations, maximum agent
turns per iteration, maximum total row additions/removals, generation concurrency,
and training device, epoch ceiling, batch size, learning rate and time ceiling.
Settings are immutable launch inputs, validated before authorization. The user
objective cannot alter budgets, acceptance, credential access or evidence roles.

One iteration means analyze development evidence, propose explicit training-row
edits, generate and validate additions, publish a new dataset version, train one
candidate, and compare it on the unchanged development benchmark. An agent turn
is one model call inside that iteration. Both have independent finite limits.
Defaults are three iterations, eight agent turns per iteration, 192 total row
changes and one generation request in flight. Concurrency is configurable from
one to sixteen. It does not bypass cumulative provider request/token/cost limits.

Quick test uses one iteration, four agent turns, eight edits, a deterministic
sample of at most 64 training rows and at most two minutes of training. It still
uses the configured agent and generator. Evaluation remains on the pinned
development benchmark; this may still take time. Quick tests never use final
holdout or qualify a model for promotion. A plain one-iteration normal run is
not called a quick test.

## Execution and recovery

The root pins baseline, starting dataset, benchmark and both provider revisions.
Each iteration pins its source dataset, development evidence, model-produced
public rationale, explicit edit proposal, generation attempts, validated dataset
version, candidate and evaluation. Source datasets and models never change.
Training schema admission belongs to the native task adapter; optimization
cannot publish arbitrary model output as valid training data. Development
observations are suitable for adaptation; protected-test rows and scores are
never agent inputs. Evaluation thresholds remain deterministic.

Every external call is reserved durably before dispatch. Completed work reuses
the same identity after restart. An interrupted request has unknown remote
outcome, consumes its reserved budget and is not described as exactly-once.
Concurrency includes outstanding reservations. Stop aborts local calls where
possible and prevents the next step from dispatching; immutable completed work
is retained. Rejected edits and failed attempts still consume their budgets.

After each development result the next iteration gets the previous result and
best eligible dataset so far. Finite limits and an explicit no-change decision
can end the loop early. Final holdout is used at most once, after all adaptive
work, for the deterministically selected candidate, only under matching launch
authority. It never feeds another iteration. The baseline is not auto-promoted.

## Presentation

One expandable run keeps Setup → Status → Report. Status displays an iteration
selector above that iteration's stage strip and activity. Finished iterations
remain inspectable; selecting history never snaps back to live updates.
Activities carry iteration, stage, timestamp and origin (Agent, Generation,
Training, Evaluation or System). Agent entries are actual model-produced public
explanations and tool decisions, not generated system captions or private
chain-of-thought. All stage activity remains paginatable without truncation.
The active iteration advances; the same strip never appears to go backwards.

Report compares iterations against the same baseline and benchmark, shows the
selected candidate and deterministic result, and labels quick tests clearly.
Provider usage is cumulative across the whole run with separate role totals;
cost is unknown unless reliably reported or calculated from pinned rates.

## Acceptance

Use deterministic agents, generation providers and trainers in ordinary tests.
Prove an evidence-driven removal and addition, a second iteration consuming the
first result, bounded concurrent dispatch, provider-budget exhaustion, invalid
native-row rejection, immutable dataset/model ancestry, no sealed leakage,
stopping and restart at each boundary, and no duplicated completed work.
Exercise the production CLI and desktop composition, not only isolated mocks.
Do not present settings in the production GUI until execution consumes them.
