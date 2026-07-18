#!/usr/bin/env python3

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ai_subagent_eval_runner", HERE / "runner.py")
RUNNER = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(RUNNER)


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.corpus = RUNNER.load_corpus(HERE / "corpus.json")

    def test_corpus_has_expected_size_categories_and_controls(self):
        self.assertEqual(len(self.corpus["tasks"]), 26)
        self.assertEqual(
            {task["category"] for task in self.corpus["tasks"]},
            RUNNER.ALLOWED_CATEGORIES,
        )
        controls = [task for task in self.corpus["tasks"] if task["negative_control"]]
        self.assertEqual(len(controls), 6)
        self.assertTrue(all(not task["parallelizable"] for task in controls))

    def test_smoke_summary_is_byte_reproducible(self):
        first = RUNNER.summarize(self.corpus, RUNNER.smoke_records(self.corpus))
        second = RUNNER.summarize(self.corpus, RUNNER.smoke_records(self.corpus))
        self.assertEqual(RUNNER.canonical_bytes(first), RUNNER.canonical_bytes(second))
        self.assertEqual(first["strategy"], "single_agent")
        self.assertEqual(first["overall"]["spawn_count_total"], 0)
        self.assertIn("not_model_measurement", first["measurement_kind"])

    def test_capture_rejects_an_incomplete_matrix(self):
        records = RUNNER.smoke_records(self.corpus)
        with self.assertRaisesRegex(RUNNER.ValidationError, "complete task/repetition matrix"):
            RUNNER.summarize(self.corpus, records[:-1])

    def test_jsonl_loader_and_summary_round_trip(self):
        records = RUNNER.smoke_records(self.corpus)
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "results.jsonl"
            result_path.write_text(
                "\n".join(json.dumps(record, sort_keys=True) for record in records) + "\n",
                encoding="utf-8",
            )
            summary = RUNNER.summarize(self.corpus, RUNNER.load_results(result_path))
        self.assertEqual(summary["overall"]["runs"], 26)
        self.assertEqual(summary["repetitions"], 1)


if __name__ == "__main__":
    unittest.main()
