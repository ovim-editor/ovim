# Ovim subagent evaluation baseline

This directory contains the Phase 0, revision-pinned evaluation contract. It
measures when delegation helps and, just as importantly, when a small or
sequential task should remain with one agent.

`corpus.json` has 26 tasks across independent architecture lookup,
cross-cutting search, log/test triage, multi-perspective review, sequential
negative controls, and small-task negative controls. The corpus is pinned to
Ovim revision `f0b1bd1`; change the corpus version when expected facts or task
wording change.

## Validate and smoke-test the runner

From the repository root:

```bash
python3 evaluations/ai-subagents/runner.py validate
python3 evaluations/ai-subagents/runner.py smoke > /tmp/ovim-subagent-runner-smoke.json
python3 evaluations/ai-subagents/test_runner.py
```

Smoke output is labeled `runner_smoke_fixture_not_model_measurement`. It proves
that corpus hashing, matrix validation, grouping, and integer metrics are
reproducible; it is deliberately not presented as model-quality evidence.

## Capture the real single-agent baseline

1. Create a clean worktree at the pinned revision.
2. Use one fixed provider/profile/model/effort and record its versions outside
   the runner output. Disable subagent tools: the baseline strategy is one root
   agent and therefore every `spawn_count` must be zero.
3. Run every task in corpus order with a fresh conversation and the same
   system/project instructions. Use at least three repetitions when variance
   matters.
4. Score success and evidence accuracy against each task's criteria. Keep the
   raw transcript and score rationale as external artifacts; do not add model
   prose to `corpus.json`.
5. Write one JSON object per task/repetition to a JSONL file, then capture the
   deterministic summary:

```bash
python3 evaluations/ai-subagents/runner.py capture \
  --results /path/to/single-agent-results.jsonl \
  --output /path/to/single-agent-baseline.json
```

Each result object has this shape:

```json
{
  "task_id": "arch-01-durable-chat-recovery",
  "run_id": "external-stable-run-id",
  "repetition": 1,
  "strategy": "single_agent",
  "measurement_kind": "measured_model_run",
  "success": true,
  "evidence_accuracy_milli": 950,
  "wall_time_ms": 42000,
  "first_useful_result_ms": 18000,
  "input_tokens": 12000,
  "output_tokens": 1800,
  "tool_calls": 14,
  "spawn_count": 0,
  "invalid_handoffs": 0,
  "failure_labels": []
}
```

All metrics use integers. Accuracy and success rates are thousandths; latency
uses milliseconds. P50/P95 use nearest-rank percentiles, category order and
JSON keys are stable, and the summary embeds the canonical SHA-256 of the
corpus. The runner rejects missing tasks, mixed strategies, duplicate
task/repetition pairs, incomplete repetition matrices, impossible timings,
and out-of-range accuracy.

## Comparing a future subagent candidate

Run the same pinned tasks, environment, repetitions, and scoring process with
subagents enabled. Preserve per-child routing, usage, and handoff artifacts in
the raw run data. The release gate is non-inferior success/evidence accuracy
with a meaningful wall-time improvement on parallelizable categories, while
the six negative controls should keep `spawn_count` at or near zero and avoid
material latency/token regression.
