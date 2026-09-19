"""Fixed local projection of Nomos training-input identities.

The program deliberately imports the same native validator and renderer used by
training qualification. It emits fingerprints and membership identities only;
model inputs and validation diagnostics never cross the process boundary.
"""

import contextlib
import hashlib
import importlib
import json
import os
from pathlib import Path
import sys


PROTOCOL = "nomos-training-inventory-v1"
MAX_BYTES = 512 * 1024 * 1024
MAX_LINE = 16 * 1024 * 1024
CONTEXT_QUESTION = "__ENCODER_GYM_CONTEXT_QUESTION_V1__"


def digest(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def fingerprint(value):
    return "sha256:" + digest(value)


def contained(root, key):
    path = (root / key).resolve(strict=True)
    if not path.is_relative_to(root) or not path.is_file():
        raise ValueError("Invalid inventory path")
    return path


def verify_sources(root, sources):
    for key, expected in sources.items():
        path = contained(root, key)
        if path.stat().st_size > MAX_LINE:
            raise ValueError("Native source exceeds inventory bound")
        actual = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError("Native source changed")


def normalized(value):
    if not isinstance(value, str) or not value.strip():
        raise ValueError("Native input cannot be rendered")
    return " ".join(value.casefold().split())


def candidate_context(candidates):
    views = []
    for candidate in candidates:
        if (not isinstance(candidate, (list, tuple)) or not candidate
                or any(not isinstance(view, str) or not view.strip() for view in candidate)):
            raise ValueError("Native candidate cannot be rendered")
        views.append(tuple(normalized(view) for view in candidate))
    return sorted(views)


def input_identity(query, candidates):
    # Match qualify_training.training_identity byte-for-byte, then add the
    # canonical prefix required by the Rust artifact contract.
    payload = [digest(normalized(query)), candidate_context(candidates)]
    return "sha256:" + digest(json.dumps(payload, ensure_ascii=False))


def label_identity(row):
    label = row.get("label")
    if not isinstance(label, dict):
        raise ValueError("Native label is missing")
    return fingerprint(json.dumps(label, sort_keys=True, ensure_ascii=False,
                                  separators=(",", ":")))


def project_member(member, render, validate, candidates):
    if not isinstance(member, dict) or set(member) != {"memberId", "row"}:
        raise ValueError("Invalid inventory member")
    member_id = member["memberId"]
    row = member["row"]
    if not isinstance(member_id, str) or not member_id or not isinstance(row, dict):
        raise ValueError("Invalid inventory member")
    if not validate(row).valid:
        raise ValueError("Native training row is invalid")
    rendered_candidates = candidates(row)
    model_input = input_identity(render(row), rendered_candidates)
    context_row = dict(row)
    context_row["question"] = CONTEXT_QUESTION
    context = input_identity(render(context_row), rendered_candidates)
    return {
        "memberId": member_id,
        "nativeContextFingerprint": context,
        "nativeModelInputFingerprint": model_input,
        "labelFingerprint": label_identity(row),
    }


def inventory(root, request, render, validate, candidates):
    if request.get("protocol") != PROTOCOL:
        raise ValueError("Unsupported inventory protocol")
    source = contained(root, request["membersKey"])
    if source.stat().st_size != request["membersBytes"] or source.stat().st_size > MAX_BYTES:
        raise ValueError("Inventory input size changed")
    digest = hashlib.sha256()
    members = []
    seen = set()
    count = 0
    with source.open("rb") as stream:
        while line := stream.readline(MAX_LINE + 1):
            if len(line) > MAX_LINE:
                raise ValueError("Inventory row exceeds bound")
            digest.update(line)
            if not line.strip():
                continue
            member = json.loads(line)
            projected = project_member(member, render, validate, candidates)
            if projected["memberId"] in seen:
                raise ValueError("Repeated inventory member")
            seen.add(projected["memberId"])
            members.append(projected)
            count += 1
    if (count != request["rows"]
            or "sha256:" + digest.hexdigest() != request["membersFingerprint"]):
        raise ValueError("Inventory population changed")
    return {
        "protocol": PROTOCOL,
        "requestFingerprint": request["fingerprint"],
        "datasetFingerprint": request["datasetFingerprint"],
        "rows": count,
        "members": members,
    }


def main():
    root = Path.cwd().resolve()
    request_path = contained(root, sys.argv[1])
    if request_path.stat().st_size > 131072:
        raise ValueError("Inventory request exceeds bound")
    request = json.loads(request_path.read_text(encoding="utf-8"))
    verify_sources(root, request["sources"])
    native_package = request.get("nativePackage")
    if native_package not in {"nomos", "fitz_tool"}:
        raise ValueError("Unsupported native package")
    with open(os.devnull, "w", encoding="utf-8") as sink, \
            contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
        dense_router = importlib.import_module(native_package + ".dense_router")
        contracts = importlib.import_module(native_package + ".generic_contracts")
        result = inventory(
            root,
            request,
            dense_router.query_document,
            contracts.validate_decision_state_v2,
            lambda row: [dense_router.candidate_views(tool)
                         for tool in dense_router.eligible_tools(row)],
        )
    verify_sources(root, request["sources"])
    output = request_path.with_name("result.json")
    with output.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, sort_keys=True, separators=(",", ":"))
        stream.flush()
        os.fsync(stream.fileno())


if __name__ == "__main__":
    try:
        main()
    except Exception:
        sys.stderr.write("Native training inventory could not verify its inputs.\n")
        sys.exit(1)
