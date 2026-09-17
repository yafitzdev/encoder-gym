"""Fixed adapter program: native schema + complete training/benchmark firewall.

No evaluator or model is invoked. Protected row text and matching identities
never leave this process; only aggregate clearance facts are written.
"""

import hashlib
import contextlib
import importlib
import json
import os
from pathlib import Path
import sys

PROTOCOL = "nomos-training-clearance-v2"
MAX_BYTES = 512 * 1024 * 1024
MAX_LINE = 16 * 1024 * 1024


def fingerprint(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def contained(root, key):
    path = (root / key).resolve(strict=True)
    if not path.is_relative_to(root) or not path.is_file():
        raise ValueError("Invalid audit path")
    return path


def rows(root, artifact):
    path = contained(root, artifact["key"])
    if artifact["bytes"] > MAX_BYTES or path.stat().st_size != artifact["bytes"]:
        raise ValueError("Input size changed")
    digest = hashlib.sha256()
    count = 0
    with path.open("rb") as stream:
        while line := stream.readline(MAX_LINE + 1):
            if len(line) > MAX_LINE:
                raise ValueError("Row exceeds audit bound")
            count += len(line)
            if count > artifact["bytes"]:
                raise ValueError("Input grew during audit")
            digest.update(line)
            if line.strip():
                row = json.loads(line)
                if not isinstance(row, dict):
                    raise ValueError("Expected native row")
                yield row
    if count != artifact["bytes"] or "sha256:" + digest.hexdigest() != artifact["fingerprint"]:
        raise ValueError("Input identity changed")


def identity(row, render):
    # Use the bound native renderer, not a second approximation of model input.
    text = render(row)
    if not isinstance(text, str) or not text.strip() or not row.get("question"):
        raise ValueError("Native input cannot be rendered")
    provenance = row.get("provenance") or {}
    if not isinstance(provenance, dict):
        raise ValueError("Invalid provenance")
    if row.get("encoder_gym_generation") is not None and not provenance.get("source_row_hash"):
        raise ValueError("Generated row has no retained template source identity")
    source = provenance.get("source_row_hash") or row.get("decision_state_id")
    group = row.get("split_group_id") or row.get("scenario_id")
    lineage = provenance.get("source_lineage") or provenance.get("trajectory_hash") or row.get("trajectory_id")
    if not isinstance(source, str) or not source.strip():
        raise ValueError("Missing source identity")
    for value in (group, lineage):
        if value is not None and (not isinstance(value, str) or not value.strip()):
            raise ValueError("Invalid grouping identity")
    # Undeclared optional groups are absent, not the hash of an empty string.
    return (
        fingerprint(text), fingerprint(" ".join(text.casefold().split())),
        fingerprint(source), fingerprint(group) if group else None,
        fingerprint(lineage) if lineage else None,
    )


def verify_sources(root, sources):
    for key, expected in sources.items():
        path = contained(root, key)
        if path.stat().st_size > MAX_LINE:
            raise ValueError("Native source exceeds audit bound")
        if "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError("Native source changed")


def training_identity(query_identity, candidates):
    # Retrieval input includes the eligible candidates, not just the query.
    # Native views exclude concrete registry IDs. Preserve the candidate
    # multiset, ignore registry order, and never use labels to hide duplicates.
    views = []
    for candidate in candidates:
        if (not isinstance(candidate, (list, tuple)) or not candidate
                or any(not isinstance(view, str) or not view.strip() for view in candidate)):
            raise ValueError("Native candidate cannot be rendered")
        views.append(tuple(" ".join(view.casefold().split()) for view in candidate))
    return fingerprint(json.dumps([query_identity, sorted(views)], ensure_ascii=False))


def audit(root, request, render, validate, candidate_inputs):
    if request["protocol"] != PROTOCOL:
        raise ValueError("Unsupported clearance protocol")
    protected = [set() for _ in range(5)]
    benchmark_rows = 0
    missing_group = missing_lineage = 0
    for artifact in request["benchmarkInputs"]:
        for row in rows(root, artifact):
            values = identity(row, render)
            for index, value in enumerate(values):
                if value is not None:
                    protected[index].add(value)
            missing_group += values[3] is None
            missing_lineage += values[4] is None
            benchmark_rows += 1
    if not benchmark_rows:
        raise ValueError("Empty benchmark population")
    seen = set()
    total = invalid = duplicates = overlaps = 0
    for row in rows(root, request["training"]):
        total += 1
        valid = validate(row).valid and row.get("accepted") is True and row.get("evaluation_partition") == "train"
        invalid += not valid
        values = identity(row, render)
        native_input = training_identity(values[1], candidate_inputs(row))
        duplicates += native_input in seen
        seen.add(native_input)
        # Benchmark isolation deliberately remains more conservative: a changed
        # candidate registry cannot excuse the same protected query or lineage.
        overlaps += any(value is not None and value in protected[index] for index, value in enumerate(values))
        missing_group += values[3] is None
        missing_lineage += values[4] is None
    if total != request["rows"]:
        raise ValueError("Incomplete training population")
    return {
        "protocol": PROTOCOL, "requestFingerprint": request["fingerprint"],
        "trainingRows": total, "benchmarkRows": benchmark_rows,
        "invalidRows": invalid, "duplicateRows": duplicates,
        "overlapRows": overlaps, "missingGroupRows": missing_group,
        "missingLineageRows": missing_lineage,
    }


def main():
    root = Path.cwd().resolve()
    request_path = contained(root, sys.argv[1])
    if request_path.stat().st_size > 131072:
        raise ValueError("Request exceeds audit bound")
    request = json.loads(request_path.read_text(encoding="utf-8"))
    verify_sources(root, request["sources"])
    native_package = request.get("nativePackage")
    if native_package not in {"nomos", "fitz_tool"}:
        raise ValueError("Unsupported native package")
    # Reuse the actual native validator and text renderer. Never pass their
    # diagnostic strings through: these can quote protected rows.
    with open(os.devnull, "w", encoding="utf-8") as sink, contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
        dense_router = importlib.import_module(native_package + ".dense_router")
        contracts = importlib.import_module(native_package + ".generic_contracts")
        result = audit(root, request, dense_router.query_document, contracts.validate_decision_state_v2,
                       lambda row: [dense_router.candidate_views(tool) for tool in dense_router.eligible_tools(row)])
    verify_sources(root, request["sources"])
    output = request_path.with_name("result.json")
    with output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, sort_keys=True)
        stream.flush()
        os.fsync(stream.fileno())


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Do not print exception text/tracebacks from protected native inputs.
        sys.stderr.write("Native training qualification could not verify its inputs.\n")
        sys.exit(1)
