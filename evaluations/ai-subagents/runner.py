#!/usr/bin/env python3
"""Validate the subagent corpus and capture deterministic evaluation summaries."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


ROOT = Path(__file__).resolve().parent
DEFAULT_CORPUS = ROOT / "corpus.json"
ALLOWED_CATEGORIES = {
    "architecture_lookup",
    "cross_cutting_search",
    "log_test_triage",
    "multi_perspective_review",
    "sequential_negative_control",
    "small_task_negative_control",
}
RESULT_INTEGER_FIELDS = (
    "repetition",
    "evidence_accuracy_milli",
    "wall_time_ms",
    "first_useful_result_ms",
    "input_tokens",
    "output_tokens",
    "tool_calls",
    "spawn_count",
    "invalid_handoffs",
)


class ValidationError(ValueError):
    pass


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def load_corpus(path: Path) -> dict[str, Any]:
    try:
        corpus = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValidationError(f"cannot read corpus {path}: {error}") from error
    validate_corpus(corpus)
    return corpus


def validate_corpus(corpus: dict[str, Any]) -> None:
    if corpus.get("version") != 1:
        raise ValidationError("corpus version must be 1")
    tasks = corpus.get("tasks")
    if not isinstance(tasks, list) or not 20 <= len(tasks) <= 30:
        raise ValidationError("corpus must contain 20 through 30 tasks")
    ids: set[str] = set()
    categories: Counter[str] = Counter()
    for index, task in enumerate(tasks):
        if not isinstance(task, dict):
            raise ValidationError(f"task {index} must be an object")
        task_id = task.get("id")
        if not isinstance(task_id, str) or not task_id or task_id in ids:
            raise ValidationError(f"task {index} has an empty or repeated ID")
        ids.add(task_id)
        category = task.get("category")
        if category not in ALLOWED_CATEGORIES:
            raise ValidationError(f"{task_id}: invalid category {category!r}")
        categories[category] += 1
        for field in ("title", "prompt"):
            if not isinstance(task.get(field), str) or not task[field].strip():
                raise ValidationError(f"{task_id}: {field} must be non-empty")
        for field in ("relevant_paths", "success_criteria"):
            values = task.get(field)
            if not isinstance(values, list) or not values or not all(
                isinstance(value, str) and value.strip() for value in values
            ):
                raise ValidationError(f"{task_id}: {field} must be non-empty strings")
        is_control = category.endswith("negative_control")
        if task.get("negative_control") is not is_control:
            raise ValidationError(f"{task_id}: negative_control disagrees with category")
        if is_control and task.get("parallelizable") is not False:
            raise ValidationError(f"{task_id}: negative controls must not be parallelizable")
        expected = "single_agent" if is_control else "parallel_optional"
        if task.get("expected_delegation") != expected:
            raise ValidationError(f"{task_id}: expected_delegation must be {expected}")
    missing = ALLOWED_CATEGORIES - categories.keys()
    if missing:
        raise ValidationError(f"corpus lacks categories: {sorted(missing)}")


def corpus_digest(corpus: dict[str, Any]) -> str:
    return hashlib.sha256(canonical_bytes(corpus)).hexdigest()


def load_results(path: Path) -> list[dict[str, Any]]:
    records = []
    try:
        for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError as error:
                raise ValidationError(f"{path}:{line_number}: {error}") from error
            if not isinstance(record, dict):
                raise ValidationError(f"{path}:{line_number}: result must be an object")
            records.append(record)
    except OSError as error:
        raise ValidationError(f"cannot read results {path}: {error}") from error
    return records


def validate_results(corpus: dict[str, Any], records: list[dict[str, Any]]) -> None:
    task_ids = {task["id"] for task in corpus["tasks"]}
    observed: set[tuple[str, int]] = set()
    strategies: set[str] = set()
    measurement_kinds: set[str] = set()
    for index, record in enumerate(records, 1):
        task_id = record.get("task_id")
        if task_id not in task_ids:
            raise ValidationError(f"result {index}: unknown task_id {task_id!r}")
        for field in ("run_id", "strategy", "measurement_kind"):
            if not isinstance(record.get(field), str) or not record[field].strip():
                raise ValidationError(f"result {index}: {field} must be a non-empty string")
        if not isinstance(record.get("success"), bool):
            raise ValidationError(f"result {index}: success must be boolean")
        for field in RESULT_INTEGER_FIELDS:
            value = record.get(field)
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                raise ValidationError(f"result {index}: {field} must be a non-negative integer")
        if record["repetition"] < 1:
            raise ValidationError(f"result {index}: repetition starts at 1")
        if record["evidence_accuracy_milli"] > 1000:
            raise ValidationError(f"result {index}: evidence_accuracy_milli exceeds 1000")
        if record["first_useful_result_ms"] > record["wall_time_ms"]:
            raise ValidationError(f"result {index}: first useful result exceeds wall time")
        labels = record.get("failure_labels")
        if not isinstance(labels, list) or not all(isinstance(label, str) for label in labels):
            raise ValidationError(f"result {index}: failure_labels must be strings")
        key = (task_id, record["repetition"])
        if key in observed:
            raise ValidationError(f"duplicate result for {task_id} repetition {key[1]}")
        observed.add(key)
        strategies.add(record["strategy"])
        measurement_kinds.add(record["measurement_kind"])
    if not records:
        raise ValidationError("results are empty")
    if len(strategies) != 1 or len(measurement_kinds) != 1:
        raise ValidationError("one capture cannot mix strategies or measurement kinds")
    repetitions = {record["repetition"] for record in records}
    expected = {(task_id, repetition) for task_id in task_ids for repetition in repetitions}
    if observed != expected:
        missing = sorted(expected - observed)
        extra = sorted(observed - expected)
        raise ValidationError(f"results are not a complete task/repetition matrix; missing={missing}, extra={extra}")


def rounded_mean(values: Iterable[int]) -> int:
    values = list(values)
    return (sum(values) + len(values) // 2) // len(values)


def percentile(values: Iterable[int], percentile_value: int) -> int:
    ordered = sorted(values)
    index = max(0, (percentile_value * len(ordered) + 99) // 100 - 1)
    return ordered[index]


def metric_summary(records: list[dict[str, Any]]) -> dict[str, Any]:
    failures = Counter(label for record in records for label in record["failure_labels"])
    return {
        "runs": len(records),
        "success_rate_milli": rounded_mean(1000 if record["success"] else 0 for record in records),
        "evidence_accuracy_milli_mean": rounded_mean(
            record["evidence_accuracy_milli"] for record in records
        ),
        "wall_time_ms_p50": percentile((record["wall_time_ms"] for record in records), 50),
        "wall_time_ms_p95": percentile((record["wall_time_ms"] for record in records), 95),
        "first_useful_result_ms_p50": percentile(
            (record["first_useful_result_ms"] for record in records), 50
        ),
        "input_tokens_total": sum(record["input_tokens"] for record in records),
        "output_tokens_total": sum(record["output_tokens"] for record in records),
        "tool_calls_total": sum(record["tool_calls"] for record in records),
        "spawn_count_total": sum(record["spawn_count"] for record in records),
        "invalid_handoffs_total": sum(record["invalid_handoffs"] for record in records),
        "failure_labels": dict(sorted(failures.items())),
    }


def summarize(corpus: dict[str, Any], records: list[dict[str, Any]]) -> dict[str, Any]:
    validate_results(corpus, records)
    category_by_task = {task["id"]: task["category"] for task in corpus["tasks"]}
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in records:
        grouped[category_by_task[record["task_id"]]].append(record)
    return {
        "version": 1,
        "corpus": {
            "name": corpus["name"],
            "version": corpus["version"],
            "pinned_revision": corpus["pinned_revision"],
            "sha256": corpus_digest(corpus),
            "task_count": len(corpus["tasks"]),
        },
        "strategy": records[0]["strategy"],
        "measurement_kind": records[0]["measurement_kind"],
        "repetitions": len({record["repetition"] for record in records}),
        "overall": metric_summary(records),
        "categories": {category: metric_summary(grouped[category]) for category in sorted(grouped)},
    }


def smoke_records(corpus: dict[str, Any]) -> list[dict[str, Any]]:
    """Runner conformance data, explicitly not a model-quality baseline."""
    records = []
    for index, task in enumerate(corpus["tasks"], 1):
        records.append(
            {
                "task_id": task["id"],
                "run_id": f"runner-smoke-{index:02d}",
                "repetition": 1,
                "strategy": "single_agent",
                "measurement_kind": "runner_smoke_fixture_not_model_measurement",
                "success": True,
                "evidence_accuracy_milli": 1000,
                "wall_time_ms": index * 100,
                "first_useful_result_ms": index * 50,
                "input_tokens": index * 10,
                "output_tokens": index * 5,
                "tool_calls": index % 4,
                "spawn_count": 0,
                "invalid_handoffs": 0,
                "failure_labels": [],
            }
        )
    return records


def write_json(value: Any, output: Path | None) -> None:
    rendered = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    if output is None:
        sys.stdout.write(rendered)
        return
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_suffix(output.suffix + ".tmp")
    temporary.write_text(rendered, encoding="utf-8")
    temporary.replace(output)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate", help="validate the pinned corpus")
    capture = subparsers.add_parser("capture", help="summarize a complete JSONL result matrix")
    capture.add_argument("--results", type=Path, required=True)
    capture.add_argument("--output", type=Path)
    smoke = subparsers.add_parser("smoke", help="exercise summary logic with non-measurement data")
    smoke.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        corpus = load_corpus(args.corpus)
        if args.command == "validate":
            print(f"valid: {len(corpus['tasks'])} tasks, sha256={corpus_digest(corpus)}")
        elif args.command == "capture":
            write_json(summarize(corpus, load_results(args.results)), args.output)
        elif args.command == "smoke":
            write_json(summarize(corpus, smoke_records(corpus)), args.output)
        return 0
    except ValidationError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
