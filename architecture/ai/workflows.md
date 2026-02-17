# AI Workflows Plan

This document proposes a workflow engine for Ovim that loads declarative
agent workflows from:

- `~/.config/ovim/workflows/*.yaml`

The goal is to support reusable multi-step AI flows (explore, plan, execute)
with structured outputs and template-driven prompts.

## Goals

1. Let users define reusable workflow graphs in YAML.
2. Support step-to-step data flow (text and structured JSON).
3. Support loop execution (`each`) for phased plans.
4. Keep runtime integrated with existing profile/context/provider pipeline.
5. Keep validation strict and errors actionable.

## Non-goals (V1)

1. No parallel branch execution.
2. No arbitrary DAG with merge nodes.
3. No custom plugin step handlers.
4. No persistence/resume across editor restarts.

V1 should be sequential with one looping primitive.

## Templating Decision: MiniJinja

Use `MiniJinja` as the template engine.

Reasons:

1. Good fit for Rust and runtime prompt rendering.
2. Supports variables, conditionals, loops, filters, arithmetic, and length.
3. Works naturally with `serde_json::Value` context.
4. Cleaner expression support than plain Handlebars for this use case.

### Syntax Choice

Adopt Jinja syntax in workflow templates:

- `{{ expr }}`
- `{% if ... %}...{% else %}...{% endif %}`
- `{% for item in items %}...{% endfor %}`

This replaces Handlebars-style `{{#if}}` and `{{#each}}` in workflow files.

## Workflow Spec (V1)

Use explicit versioning and a typed schema.

```yaml
version: 1
name: refactor
description: Multi-step refactoring workflow

inputs:
  refactoring_objective:
    type: string
    required: false
    description: The target of the refactor

steps:
  - id: exploration_report
    kind: prompt
    mode: readonly
    context: query
    prompt: |
      {% if inputs.refactoring_objective %}
      Your goal is to identify refactoring opportunities according to:
      {{ inputs.refactoring_objective }}
      {% else %}
      Your goal is to identify refactoring opportunities in the codebase.
      {% endif %}
    output:
      format: text

  - id: roadmap
    kind: prompt
    mode: readonly
    context: query
    prompt: |
      Based on this exploration report, create a phased roadmap:
      {{ steps.exploration_report.output }}
    output:
      format: json
      schema:
        type: array
        items:
          type: object
          additionalProperties: false
          required: [plan, summary]
          properties:
            plan: { type: string }
            summary: { type: string }

  - id: roadmap_execution
    kind: each
    items: steps.roadmap.output
    as: plan
    index: index
    step:
      kind: prompt
      mode: readonly
      context: chat
      prompt: |
        # Roadmap
        {% for s in steps.roadmap.output %}
        {{ loop.index }}. {{ s.summary }}
        {% endfor %}

        # Current objective (phase {{ index + 1 }} / {{ steps.roadmap.output|length }})
        {{ plan.plan }}
      output:
        format: text
```

## Schema Guidance

The original shorthand:

```yaml
schema:
  - plan: string
    summary: string
```

is ambiguous. Use JSON Schema in YAML form for determinism and validation.

Benefits:

1. Clear and machine-checkable contract.
2. Better error messages per field/path.
3. Extensible for strict mode (`additionalProperties`, enums, bounds).

## Runtime Data Model

Template context at render time should be a JSON object with predictable keys:

```text
workflow: metadata (name/version)
inputs: user-provided workflow inputs
steps: map keyed by step id
state: runtime metadata (current_step, timestamps, counters)
```

For `each` bodies:

```text
<as>: current item
<index>: current zero-based index
```

`steps.<id>.output` stores either:

1. text (`String`) when `output.format = text`
2. parsed JSON (`serde_json::Value`) when `output.format = json`

## Execution Semantics

### Load and Compile

1. Discover YAML files in workflow dirs.
2. Parse YAML into `WorkflowSpec`.
3. Validate structural invariants:
   - unique workflow name
   - unique step ids
   - `each.items` references existing JSON value
   - disallow forward references in V1
4. Precompile MiniJinja templates for each prompt.

### Run

1. Create `WorkflowRunState` with input values and empty step outputs.
2. Execute steps in order.
3. `prompt` step:
   - render template
   - resolve profile via existing context/profile mapping
   - run through existing AI request path
   - parse output (`text` or `json`)
   - validate JSON against schema if provided
4. `each` step:
   - evaluate `items` expression to array
   - for each item, execute nested `step`
   - collect outputs as array
5. Emit per-step status updates to UI/status line.

### Errors

Return actionable failures with workflow/step context:

1. `template_error`: include step id and missing variable path.
2. `schema_validation_error`: include JSON pointer and expected type.
3. `provider_error`: include profile/provider/model and message.
4. `runtime_error`: invalid `items` expression, non-array loop target, etc.

## Integration Plan in Current Codebase

### New Modules

Add:

- `ovim-core/src/ai/workflow/mod.rs`
- `ovim-core/src/ai/workflow/spec.rs`
- `ovim-core/src/ai/workflow/loader.rs`
- `ovim-core/src/ai/workflow/template.rs`
- `ovim-core/src/ai/workflow/engine.rs`
- `ovim-core/src/ai/workflow/schema.rs`

Update export surface in `ovim-core/src/ai/mod.rs`.

### Dependencies

Add to `ovim-core/Cargo.toml`:

1. `serde_yaml` for YAML parsing.
2. `minijinja` for template rendering.
3. `jsonschema` (or equivalent) for schema validation.

### Editor State

Extend `AiState` (`ovim-core/src/editor/ai_state.rs`) with:

1. loaded workflow registry (`HashMap<String, WorkflowSpec>`).
2. active workflow runs and run statuses.
3. pending workflow job handles/channels.

### Event Loop

In `ovim/src/event_loop.rs`, add workflow poll call near existing AI polls:

1. `poll_pending_ai_jobs()`
2. `poll_pending_ai_chat_job()`
3. `poll_pending_workflow_jobs()` (new)

### Command Surface

Extend command handler in `ovim-core/src/commands.rs`:

1. `:workflow list`
2. `:workflow run <name> [k=v ...]`
3. `:workflow reload`
4. `:workflow status`

### Lua Surface

Extend `vim.ai` API:

1. `vim.ai.workflows.list()`
2. `vim.ai.workflows.run(name, opts)`
3. `vim.ai.workflows.reload()`

Bridge through `ovim-core/src/lua/ai_api.rs`,
`ovim-core/src/lua/editor_bridge.rs`, and
`ovim-core/src/editor/lua_integration.rs`.

## Phased Rollout

### Phase 1: Spec, Loader, Validation

1. YAML spec structs + parser.
2. MiniJinja template compile checks.
3. Command `:workflow list` and `:workflow reload`.

### Phase 2: Prompt Step Executor

1. `prompt` step execution only.
2. Text output mode.
3. Command `:workflow run`.

### Phase 3: JSON Output + Schema

1. JSON parse and validation.
2. Rich schema error reporting.
3. Retry policy hooks for schema failures.

### Phase 4: Looping (`each`)

1. `each` step support.
2. Nested step execution and output aggregation.
3. Runtime guards (max loop items, max rendered prompt size).

### Phase 5: UI and Lua API

1. Run status in chat/status area.
2. Lua workflow API.
3. Keymap-friendly execution hooks.

## Testing Plan

1. Unit tests for YAML parse and structural validation.
2. Unit tests for MiniJinja render context and missing-variable errors.
3. Unit tests for schema validation success/failure.
4. Engine tests for step ordering and `each` output collection.
5. Integration tests for command path `:workflow run`.
6. Regression tests for malformed workflows and actionable errors.

## Open Decisions

1. Should V1 allow forward step references?
2. Should `each.items` be an expression or only a dotted path?
3. Should workflow runs write transcript artifacts to disk?
4. Should step-level profile overrides be allowed in V1?

Current recommendation:

1. No forward refs.
2. Dotted path only in V1 (simpler validation).
3. No artifact persistence in V1.
4. Allow optional per-step `profile` override in V1.
