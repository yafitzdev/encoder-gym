# Dimension and initial-allocation engine

The engine turns a dataset schema into a finite, explicit set of generation
cells and assigns one absolute accepted-row target to every cell. It is pure
and deterministic: no LLM chooses the distribution, and generation providers
do not participate in allocation.

## Inspect the generation space

Before expanding a large Cartesian product, inspect its cardinality:

```text
synth plan describe <DATASET_ID>
synth plan preview <DATASET_ID>
```

`describe` reports label count, each dimension's value count, and the resulting
cell count without materializing the cells. Dataset definitions are limited to
100,000 cells so an accidental Cartesian explosion cannot exhaust the local
process.

## Allocate an exact total

```text
synth allocation preview <DATASET_ID> --total-rows 20000 --explain
synth allocation create <DATASET_ID> --total-rows 20000 --explain
```

`total_rows` is the desired absolute accepted-row total, not the number of new
rows to request. Persisted accepted coverage is subtracted per cell. With
`--reserved-rows 2000`, only 18,000 rows are assigned to the initial plan; the
remaining 2,000 stay outside that plan for a later bounded iteration.

Available policies are:

- `balanced`: equal effective weight for every active complete cell;
- `weighted`: multiply optional label and dimension-value weights for each
  cell, then distribute the exact integer total by deterministic largest
  remainder;
- `minimum-then-weighted`: apply a floor to every active cell before weighted
  distribution; and
- `explicit`: require a complete absolute target for every cell.

"Optimal" therefore always means the selected, inspectable policy. It does not
mean an opaque LLM or learned optimizer has inferred a distribution. Integer
rounding can make realized target shares differ slightly from effective weight
shares; `--explain` shows both.

## Concise constraint rules

`--constraints` accepts the original exact-cell constraint documents or a TOML
or JSON document containing partial selector rules. Start from
[`examples/allocation-constraints.toml`](../examples/allocation-constraints.toml).
A selector may name a label, any subset of dimensions, or both. An empty
selector matches every cell.

Overlapping rules combine conservatively per matched cell:

- the highest minimum wins;
- the lowest maximum wins; and
- any exclusion wins.

Unknown labels, dimensions, or values fail before allocation. Repeating the
same selector is rejected as an operator mistake. If combined constraints are
infeasible, preview returns structured issues and `create` refuses to persist a
plan rather than silently changing the requested total or a bound.

## Explain and trace

`--explain` adds compact summaries for every label and every dimension value,
including cell count, current coverage, final target, additional work, target
share, and effective weight share. A persisted allocation can be summarized or
traced later:

```text
synth allocation explain <ALLOCATION_ID>
synth provenance initial-allocation <ALLOCATION_ID>
synth provenance generation-plan <PLAN_ID>
```

`allocation create` atomically stores the immutable, fingerprinted allocation
and its ordinary Slice 1 generation plan. The persisted allocation contains
the compiled effective constraint on each cell, so its exact decision can be
reproduced even if the source rule file later changes.
