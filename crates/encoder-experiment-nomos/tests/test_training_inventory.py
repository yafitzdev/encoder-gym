"""Offline contract tests for the fixed native inventory projection."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest


SPEC = importlib.util.spec_from_file_location(
    "training_inventory",
    Path(__file__).parents[1] / "src" / "inspect_training_inventory.py",
)
projection = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(projection)


def row(question, label="a", state="one", candidates=("read", "write")):
    return {
        "question": question,
        "label": {"acceptable_tools": [label]},
        "state": state,
        "candidates": list(candidates),
    }


class TrainingInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    @staticmethod
    def render(value):
        return f"state={value['state']} question={value['question']}"

    @staticmethod
    def candidates(value):
        return [[candidate] for candidate in value["candidates"]]

    def request(self, members):
        content = b"".join(
            json.dumps(member, separators=(",", ":")).encode("utf-8") + b"\n"
            for member in members
        )
        (self.root / "members.jsonl").write_bytes(content)
        return {
            "protocol": projection.PROTOCOL,
            "fingerprint": "sha256:" + "1" * 64,
            "datasetFingerprint": "sha256:" + "2" * 64,
            "membersKey": "members.jsonl",
            "membersBytes": len(content),
            "membersFingerprint": "sha256:" + hashlib.sha256(content).hexdigest(),
            "rows": len(members),
        }

    def inspect(self, members):
        return projection.inventory(
            self.root,
            self.request(members),
            self.render,
            lambda _: SimpleNamespace(valid=True),
            self.candidates,
        )

    def test_context_ignores_question_but_keeps_state_and_candidate_pool(self):
        result = self.inspect([
            {"memberId": "a", "row": row("first")},
            {"memberId": "b", "row": row("second")},
            {"memberId": "c", "row": row("first", state="two")},
            {"memberId": "d", "row": row("first", candidates=("read", "search"))},
        ])
        members = {member["memberId"]: member for member in result["members"]}
        self.assertEqual(
            members["a"]["nativeContextFingerprint"],
            members["b"]["nativeContextFingerprint"],
        )
        self.assertNotEqual(
            members["a"]["nativeModelInputFingerprint"],
            members["b"]["nativeModelInputFingerprint"],
        )
        self.assertNotEqual(
            members["a"]["nativeContextFingerprint"],
            members["c"]["nativeContextFingerprint"],
        )
        self.assertNotEqual(
            members["a"]["nativeContextFingerprint"],
            members["d"]["nativeContextFingerprint"],
        )

    def test_same_native_input_with_different_labels_is_visible_as_a_conflict(self):
        result = self.inspect([
            {"memberId": "a", "row": row("same", label="a")},
            {"memberId": "b", "row": row("same", label="b")},
        ])
        first, second = result["members"]
        self.assertEqual(
            first["nativeModelInputFingerprint"],
            second["nativeModelInputFingerprint"],
        )
        self.assertNotEqual(first["labelFingerprint"], second["labelFingerprint"])

    def test_invalid_incomplete_or_repeated_populations_fail_closed(self):
        members = [{"memberId": "a", "row": row("one")}]
        request = self.request(members)
        request["rows"] = 2
        with self.assertRaises(ValueError):
            projection.inventory(
                self.root,
                request,
                self.render,
                lambda _: SimpleNamespace(valid=True),
                self.candidates,
            )
        repeated = [members[0], members[0]]
        with self.assertRaises(ValueError):
            self.inspect(repeated)
        with self.assertRaises(ValueError):
            projection.inventory(
                self.root,
                self.request(members),
                self.render,
                lambda _: SimpleNamespace(valid=False),
                self.candidates,
            )


if __name__ == "__main__":
    unittest.main()
