"""Offline tests of the fixed audit program; native libraries are injected only
into the pure audit function, never into its production main entry point."""
import hashlib
import importlib.util
import json
import subprocess
import sys
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

SPEC = importlib.util.spec_from_file_location("clearance", Path(__file__).parents[1] / "src" / "qualify_training.py")
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


def row(identity, question, **extra):
    return {"decision_state_id": identity, "question": question,
            "accepted": True, "evaluation_partition": "train", **extra}


class ClearanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def artifact(self, key, values):
        data = ("\n".join(json.dumps(value) for value in values) + "\n").encode()
        (self.root / key).write_bytes(data)
        return {"key": key, "bytes": len(data), "fingerprint": "sha256:" + hashlib.sha256(data).hexdigest()}

    def request(self, training, protected):
        return {"protocol": audit.PROTOCOL, "fingerprint": "pinned-request", "rows": len(training),
                "training": self.artifact("training.jsonl", training),
                "benchmarkInputs": [self.artifact("benchmark.jsonl", protected)]}

    def run_audit(self, request):
        return audit.audit(self.root, request, lambda value: value["question"],
                           lambda value: SimpleNamespace(valid=not value.get("invalid")),
                           lambda value: [tool["views"] for tool in value.get("candidates", [])])

    def test_same_query_with_different_native_candidates_is_not_duplicate(self):
        training = [row("a", "same query", candidates=[{"views": ["read documents"]}]),
                    row("b", "same query", candidates=[{"views": ["search web"]}])]
        result = self.run_audit(self.request(training, [row("z", "protected")]))
        self.assertEqual(result["duplicateRows"], 0)

    def test_candidate_identity_and_order_cannot_hide_duplicate_native_inputs(self):
        training = [row("a", "Same query", candidates=[
                        {"id": "one", "views": ["Read documents", "Purpose: read"]},
                        {"id": "two", "views": ["Search web"]}]),
                    row("b", " same   QUERY ", candidates=[
                        {"id": "renamed-two", "views": ["search WEB"]},
                        {"id": "renamed-one", "views": [" read documents ", "Purpose: read"]}])]
        result = self.run_audit(self.request(training, [row("z", "protected")]))
        self.assertEqual(result["duplicateRows"], 1)

    def test_different_supervision_cannot_hide_duplicate_native_inputs(self):
        training = [row("a", "same", label={"acceptable_tools": ["one"]}),
                    row("b", "same", label={"acceptable_tools": ["two"]})]
        result = self.run_audit(self.request(training, [row("z", "protected")]))
        self.assertEqual(result["duplicateRows"], 1)

    def test_benchmark_query_overlap_ignores_different_candidate_context(self):
        training = [row("a", "same query", candidates=[{"views": ["read documents"]}])]
        protected = [row("z", " SAME QUERY ", candidates=[{"views": ["search web"]}])]
        result = self.run_audit(self.request(training, protected))
        self.assertEqual(result["overlapRows"], 1)

    def test_invalid_native_candidate_rendering_fails_closed(self):
        for views in ([None], [""], "not a sequence of views"):
            with self.subTest(views=views), self.assertRaises(ValueError):
                self.run_audit(self.request([row("a", "query", candidates=[{"views": views}])],
                                           [row("z", "protected")]))

    def test_legacy_protocol_cannot_reuse_new_duplicate_rules(self):
        request = self.request([row("a", "alpha")], [row("z", "protected")])
        request["protocol"] = "nomos-training-clearance-v1"
        with self.assertRaises(ValueError):
            self.run_audit(request)

    def test_main_uses_native_eligible_candidate_views(self):
        request = self.request([
            row("a", "same", candidates=["one"]),
            row("b", "same", candidates=["two"]),
            row("c", "same", candidates=["one"], label={"different": True}),
        ], [row("z", "protected")])
        package = self.root / "nomos"
        package.mkdir()
        sources = {
            "dense_router.py": "def query_document(row): return row['question']\n"
                               "def eligible_tools(row): return row['candidates']\n"
                               "def candidate_views(tool): return [tool]\n",
            "generic_contracts.py": "from types import SimpleNamespace\n"
                                    "def validate_decision_state_v2(row): return SimpleNamespace(valid=True)\n",
        }
        request["nativePackage"] = "nomos"
        request["sources"] = {}
        for name, code in sources.items():
            path = package / name
            path.write_text(code, encoding="utf-8")
            request["sources"]["nomos/" + name] = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        (self.root / "request.json").write_text(json.dumps(request), encoding="utf-8")
        completed = subprocess.run([sys.executable, "-B", "-c", Path(SPEC.origin).read_text(encoding="utf-8"), "request.json"],
                                   cwd=self.root, capture_output=True, text=True, timeout=10)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads((self.root / "result.json").read_text(encoding="utf-8"))
        self.assertEqual(result["duplicateRows"], 1)
        self.assertEqual(result["overlapRows"], 0)

    def test_complete_native_validation_and_no_empty_group_collisions(self):
        request = self.request([row("a", "alpha"), row("b", "beta")], [row("z", "protected")])
        result = self.run_audit(request)
        self.assertEqual(result["trainingRows"], 2)
        self.assertEqual(result["overlapRows"], 0)
        self.assertEqual(result["missingGroupRows"], 3)
        self.assertEqual(result["missingLineageRows"], 3)
        self.assertNotIn("protected", json.dumps(result))

    def test_all_five_identity_checks_and_protected_payload_exclusion(self):
        training = [row("a", "exact"), row("b", "  Case   Fold  "),
                    row("source", "third"), row("d", "fourth", scenario_id="shared-group"),
                    row("e", "fifth", provenance={"source_lineage": "shared-lineage"})]
        protected = [row("p", "exact"), row("q", "case fold"),
                     row("source", "different"), row("s", "another", split_group_id="shared-group"),
                     row("t", "NEVER_DISCLOSE_HOLDOUT", trajectory_id="shared-lineage")]
        result = self.run_audit(self.request(training, protected))
        self.assertEqual(result["overlapRows"], 5)
        self.assertNotIn("NEVER_DISCLOSE_HOLDOUT", json.dumps(result))
        self.assertNotIn("shared-lineage", json.dumps(result))

    def test_invalid_and_duplicate_rows_anywhere_in_population(self):
        request = self.request([row("a", "Alpha"), row("b", " alpha ", invalid=True),
                                row("c", "gamma", evaluation_partition="test")], [row("z", "protected")])
        result = self.run_audit(request)
        self.assertEqual(result["invalidRows"], 2)
        self.assertEqual(result["duplicateRows"], 1)

    def test_full_file_hash_is_checked_even_for_unmatched_last_row(self):
        request = self.request([row("a", "alpha")], [row("z", "protected")])
        path = self.root / "benchmark.jsonl"
        path.write_bytes(path.read_bytes().replace(b"protected", b"substitut"))
        with self.assertRaises(ValueError):
            self.run_audit(request)

    def test_incomplete_training_and_missing_source_fail_closed(self):
        request = self.request([row("a", "alpha")], [row("z", "protected")])
        request["rows"] = 2
        with self.assertRaises(ValueError):
            self.run_audit(request)
        request = self.request([row("a", "alpha")], [{"question": "protected"}])
        with self.assertRaises(ValueError):
            self.run_audit(request)

    def test_result_never_contains_native_validation_diagnostic_text(self):
        request = self.request([row("a", "alpha")], [row("z", "protected")])
        result = audit.audit(self.root, request, lambda value: value["question"],
                             lambda value: SimpleNamespace(valid=False, issues=["SECRET ROW"]),
                             lambda value: [])
        self.assertNotIn("SECRET ROW", json.dumps(result))

    def test_generated_paraphrase_cannot_erase_template_source_overlap(self):
        request = self.request([row("new-generated-id", "New wording", provenance={"source_row_hash": "original"})],
                               [row("original", "NEVER_DISCLOSE_HOLDOUT")])
        self.assertEqual(self.run_audit(request)["overlapRows"], 1)

    def test_native_exception_and_stderr_cannot_disclose_protected_data(self):
        request = self.request([row("a", "alpha")], [row("z", "protected")])
        package = self.root / "nomos"
        package.mkdir()
        code = "import sys\nprint('NEVER_DISCLOSE_HOLDOUT', file=sys.stderr)\nraise ValueError('NEVER_DISCLOSE_HOLDOUT')\n"
        path = package / "dense_router.py"
        path.write_text(code, encoding="utf-8")
        request["nativePackage"] = "nomos"
        request["sources"] = {"nomos/dense_router.py": "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()}
        (self.root / "request.json").write_text(json.dumps(request), encoding="utf-8")
        completed = subprocess.run([sys.executable, "-B", "-c", Path(SPEC.origin).read_text(encoding="utf-8"), "request.json"],
                                   cwd=self.root, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual(completed.stdout, "")
        self.assertEqual(completed.stderr.strip(), "Native training qualification could not verify its inputs.")
        self.assertFalse((self.root / "result.json").exists())

    def test_pre_rename_native_package_remains_redacted_and_importable(self):
        request = self.request([row("a", "alpha")], [row("z", "protected")])
        package = self.root / "fitz_tool"
        package.mkdir()
        code = "import sys\nprint('NEVER_DISCLOSE_HOLDOUT', file=sys.stderr)\nraise ValueError('NEVER_DISCLOSE_HOLDOUT')\n"
        path = package / "dense_router.py"
        path.write_text(code, encoding="utf-8")
        request["nativePackage"] = "fitz_tool"
        request["sources"] = {"fitz_tool/dense_router.py": "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()}
        (self.root / "request.json").write_text(json.dumps(request), encoding="utf-8")
        completed = subprocess.run([sys.executable, "-B", "-c", Path(SPEC.origin).read_text(encoding="utf-8"), "request.json"],
                                   cwd=self.root, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual(completed.stdout, "")
        self.assertEqual(completed.stderr.strip(), "Native training qualification could not verify its inputs.")
        self.assertFalse((self.root / "result.json").exists())

    def test_legacy_generated_rows_without_retained_ancestry_fail_closed(self):
        request = self.request([row("new-id", "New wording", encoder_gym_generation={"templateRowId": "original"})],
                               [row("original", "protected")])
        with self.assertRaises(ValueError):
            self.run_audit(request)


if __name__ == "__main__":
    unittest.main()
