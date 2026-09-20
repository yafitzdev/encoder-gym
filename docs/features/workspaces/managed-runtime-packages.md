# Managed runtime packages

## Problem

A managed project is not portable if its active scientific binding still
launches an adapter checkout or Python environment from an arbitrary external
path. Renaming that checkout, deleting its virtual environment, changing its
Git worktree, or opening the project on another path then breaks an otherwise
intact run.

The external Nomos checkout and interpreter are onboarding inputs only. New
bindings must not retain either path as an execution dependency.

## Canonical layout

Each published runtime is content-addressed below the project:

```text
<project>/
  runtimes/
    nomos/
      <package-sha256>/
        runtime-package.json
        workspace/
          .git/
          encoder-gym-experiment.json
          nomos/ or fitz_tool/
          tools/
          data/
          artifacts/
          runs/
        python/
          runtime/
            python.exe
            Lib/
```

`workspace/` is the adapter working directory. Its clean, no-remote Git
snapshot, pinned inputs, model trees, and pre-existing resumable outputs are
copied without links. Root scientific SQLite files and disposable caches are
not copied: the authoritative scientific database is already contained under
the managed project's `runs/` directory.

`python/runtime/` is built from the selected interpreter's base installation
plus that environment's site-packages. The copied interpreter must pass the
same offline capability check and all resolved module/search paths must remain
inside the package. This removes dependencies on both the selected virtual
environment and its base Python installation.

## Identity and binding

`runtime-package.json` records:

- adapter protocol and configuration identity;
- bound scientific project and source revision/fingerprint;
- canonical package-relative workspace and executable paths;
- sorted file inventories, byte counts, and SHA-256 identities for the runtime
  seed and Python runtime; and
- one reproducible package fingerprint.

The package directory name is that fingerprint. A managed
`ScientificBinding` records project-relative runtime and executable paths plus
the exact package identity. Absolute paths are invalid for managed bindings.
Every command resolves and contains those paths beneath the current project
root before opening Nomos. Moving the whole project therefore preserves the
binding.

Existing `external-isolated` bindings remain readable so historical evidence
and the current scientific store can be migrated. `bind-nomos` verifies that
legacy source, publishes the managed package, reuses or imports the verified
contained store, then appends a new `managed` binding. It never rewrites the old
binding or historical runs.

## Mutation and verification

Package publication is staged below `runtimes/nomos/` and atomically renamed.
Files must be regular files/directories; symlinks, junctions, reparse points,
and hard-link shortcuts are not accepted. Copying streams and hashes each
immutable file, then the copied Nomos backend reproduces the exact bound
project and the copied Python performs its offline capability check before the
binding can become active.

Nomos may append adapter-owned outputs below the packaged workspace during an
authorized run. Git-tracked source, pinned input files, existing model/output
artifacts, and Python files remain content-bound. Git's mutable index and new
run outputs are not treated as source identity; Nomos's normal manifests and
scientific journal remain authoritative for them.

`workspace verify` additionally rehashes every inventoried package file,
rechecks Python capabilities, and reproduces the bound Nomos project. Normal
status reads use shallow package identity and do not scan the entire Python
installation.

## Operator flow

`preview-nomos-binding` remains read-only and now reports the number of files
and bytes that will be copied. `prepare-nomos-python` still repairs only the
explicitly selected source environment after separate network authorization.
`bind-nomos` performs the managed copy and append-only rebind. After it
succeeds, the source checkout and selected virtual environment may be renamed
or removed without affecting new or resumed managed runs.

The desktop uses these same CLI boundaries. It presents the copy size before
confirmation and labels the operation as copying into the project, not merely
connecting an external runtime.
