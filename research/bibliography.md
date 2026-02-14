# Bibliography

Papers, blog posts, and documentation referenced in this research folder.

## Papers

### Inference-Time Scaling

- **Inference-Time Scaling for Complex Tasks: Where We Stand and What Lies Ahead**
  Microsoft Research, arXiv 2504.00294, Apr 2025
  https://arxiv.org/abs/2504.00294
  Survey of inference-time scaling methods. Finds advantages diminish as
  problem simplicity increases.

- **S*: Test Time Scaling for Code Generation**
  arXiv 2502.14382, Feb 2025
  https://arxiv.org/html/2502.14382v1
  Non-reasoning models can surpass reasoning models with the right
  test-time scaling strategy.

- **Are More Tokens Rational? Inference-Time Scaling as Adaptive Resource Rationality**
  arXiv 2602.10329, Feb 2026
  https://arxiv.org/html/2602.10329
  Models don't always benefit from more tokens. Adaptive allocation is key.

- **Faster and Better LLMs via Latency-Aware Test-Time Scaling**
  EMNLP 2025 Findings
  https://aclanthology.org/2025.findings-emnlp.928.pdf
  Latency-optimal scaling via joint parallelism optimization.

- **DaJ: Data-Reweighted LLM Judge for Test-Time Scaling in Code Generation**
  arXiv 2601.22230, Jan 2026
  https://arxiv.org/html/2601.22230
  State-of-the-art on LiveCodeBench and BigCodeBench via test-time scaling.

- **A Survey of Scaling in Large Language Model Reasoning**
  arXiv 2504.02181, Apr 2025
  https://arxiv.org/html/2504.02181v1
  Comprehensive survey of sequential and parallel scaling methods.

- **How Does LLM Reasoning Work for Code? A Survey and a Call to Action**
  arXiv 2506.13932, Jun 2025
  https://arxiv.org/html/2506.13932v1
  Survey of reasoning approaches for code tasks specifically.

- **Scaling Test-time Compute for LLM Agents**
  arXiv 2506.12928, Jun 2025
  https://arxiv.org/abs/2506.12928
  First systematic study of test-time scaling for agents. BoN with
  list-wise verification achieves 63.03% on GAIA. Multi-model mixing
  reaches 74.55% Pass@4.

- **Scaling LLM Test-Time Compute Optimally Can be More Effective than Scaling Parameters**
  OpenReview (NeurIPS 2024)
  https://openreview.net/forum?id=4FWAwZtd2n
  Test-time compute scaling can outperform parameter scaling for reasoning.

### Code Editing and Generation

- **Evaluating Code Reasoning Abilities of LLMs Under Real-World Settings**
  arXiv 2512.14917, Dec 2025
  https://arxiv.org/html/2512.14917v1
  Evaluates GPT-4.1, o4-mini, Gemini, DeepSeek on code reasoning tasks.

- **Let the Code LLM Edit Itself When You Edit the Code**
  OpenReview (ICLR submission)
  https://openreview.net/forum?id=zqzsZ5cXbB
  Research on self-editing LLMs for code.

- **Diff-XYZ: A Benchmark for Evaluating Diff Understanding**
  arXiv 2510.12487, DL4Code Workshop, Dec 2025
  https://arxiv.org/html/2510.12487v2
  Benchmark comparing unified diff, search-replace, and other formats
  across apply, anti-apply, and generation tasks. Search-replace wins
  overall for large models. 1,000 real-world edits from CommitPackFT.

- **ROCODE: Integrating Backtracking Mechanism and Program Analysis in LLMs for Code Generation**
  ICSE 2025, arXiv 2411.07112
  https://arxiv.org/abs/2411.07112
  Mid-generation error detection and backtracking. 99.1% compilation
  pass rate, 23.8% higher test pass rate, 19.3% token reduction.

- **Prompting LLMs for Code Editing: Struggles and Remedies**
  arXiv 2504.20196, Apr 2025
  https://arxiv.org/html/2504.20196v2
  Identifies 5 categories of missing info in developer prompts.
  AutoPrompter achieves 27% improvement by inferring missing context.

- **SWE-Pruner: Self-Adaptive Context Pruning for Coding Agents**
  arXiv 2601.16746, Jan 2026
  https://arxiv.org/pdf/2601.16746
  Dynamic goal hints for context pruning. 39% token reduction on
  SWE-Bench Verified with Claude Sonnet 4.5, 26% fewer rounds.

- **A Survey on Code Generation with LLM-based Agents**
  arXiv 2508.00083, Aug 2025
  https://arxiv.org/pdf/2508.00083
  Comprehensive survey of agent-based code generation approaches.

### Agent Design and Prompting

- **ReAct: Synergizing Reasoning and Acting in Language Models**
  Yao et al., arXiv 2210.03629, Oct 2022
  https://arxiv.org/abs/2210.03629
  Original ReAct paper. Interleaves reasoning traces with actions.

- **On the Brittle Foundations of ReAct Prompting for Agentic LLMs**
  Verma et al., arXiv 2405.13966, May 2024
  https://arxiv.org/abs/2405.13966
  Performance driven by exemplar-query similarity, not reasoning
  interleaving. Placebo guidance performs comparably to crafted reasoning.

- **RouteLLM: Learning to Route LLMs with Preference Data**
  ICLR 2025 (UC Berkeley, Anyscale, Canva)
  https://proceedings.iclr.cc/paper_files/paper/2025/file/5503a7c69d48a2f86fc00b3dc09de686-Paper-Conference.pdf
  85% cost reduction with 95% of GPT-4 quality via learned routing.

### Constrained Decoding

- **Flexible and Efficient Grammar-Constrained Decoding**
  ICML 2025, arXiv 2502.05111
  https://arxiv.org/abs/2502.05111
  17.71× faster preprocessing for grammar-constrained decoding.

- **Grammar-Constrained Decoding for Structured NLP Tasks without Finetuning**
  arXiv 2305.13971, May 2023
  https://arxiv.org/abs/2305.13971
  CFG-based output constraints for LLMs.

## Blog Posts and Articles

### Harness Design

- **I Improved 15 LLMs at Coding in One Afternoon. Only the Harness Changed.**
  Can.ac, Feb 12, 2026
  https://blog.can.ac/2026/02/12/the-harness-problem/
  Key finding: edit format matters as much as model quality. Introduces
  "hashline" format. Tests 16 models on 180 tasks. Grok Code Fast:
  6.7% → 68.3% by changing format alone.

- **Code Surgery: How AI Assistants Make Precise Edits to Your Files**
  Fabian Hertwig, 2025
  https://fabianhertwig.com/blog/coding-assistants-file-edits/
  Detailed comparison of apply_patch, str_replace, whole file, neural
  merge, and speculative editing across Codex, Claude Code, Aider, Cursor,
  RooCode, OpenHands.

- **How do LLM-powered tools in IDEs edit files?**
  GitHub Community Discussion #171782
  https://github.com/orgs/community/discussions/171782
  Community discussion comparing IDE edit approaches.

### Agent Design

- **Building Effective Agents**
  Anthropic Research, Dec 2024
  https://www.anthropic.com/research/building-effective-agents
  Five patterns: prompt chaining, routing, parallelization,
  orchestrator-workers, evaluator-optimizer. Key principle: start simple.

- **Writing Tools for Agents**
  Anthropic Engineering, 2025
  https://www.anthropic.com/engineering/writing-tools-for-agents
  Tool design as agent-computer interface. Actionable errors, token
  efficiency, non-overlapping tools, namespace conventions.

- **Effective Context Engineering for AI Agents**
  Anthropic Engineering, 2025
  https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
  Write/Select/Compress/Isolate framework. Context as finite resource.
  Observation masking, structured note-taking, sub-agent architectures.

### Context Management

- **Cutting Through the Noise: Smarter Context Management for LLM-Powered Agents**
  JetBrains Research Blog, Dec 2025
  https://blog.jetbrains.com/research/2025/12/efficient-context-management/
  Observation masking > LLM summarization for coding agents. 52% cheaper,
  2.6% better solve rates. 15% trajectory elongation problem with summarization.

### Workflows and Best Practices

- **My LLM Coding Workflow Going into 2026**
  Addy Osmani, Dec 2025
  https://medium.com/@addyosmani/my-llm-coding-workflow-going-into-2026-52fe1681325e
  Practical workflow patterns for LLM-assisted coding.

- **The Best AI Models for Coding: Accuracy, Integration, and Developer Fit**
  JetBrains AI Blog, Feb 2026
  https://blog.jetbrains.com/ai/2026/02/the-best-ai-models-for-coding-accuracy-integration-and-developer-fit/
  Based on JetBrains Developer Ecosystem Report 2025. 85% of developers
  regularly use AI tools for coding.

### Model-Specific

- **GPT-5.2 Prompting Guide: The 2026 Playbook**
  Atlabs AI, 2026
  https://www.atlabs.ai/blog/gpt-5.2-prompting-guide-the-2026-playbook-for-developers-agents
  CTCO Framework, reasoning effort tuning, scope discipline.

- **GPT-5.2 Is OpenAI's "Code Red" Counterpunch, and It Mostly Lands**
  Turing College, 2026
  https://www.turingcollege.com/blog/gpt-5-2-review
  Hands-on review with benchmark analysis. SWE-Bench Pro: 55.6%.

### Routing and Cost Optimization

- **Intelligent LLM Routing: How Multi-Model AI Cuts Costs by 85%**
  Swfte AI, 2025
  https://www.swfte.com/blog/intelligent-llm-routing-multi-model-ai
  Pre-generation and cascade routing strategies for cost optimization.

- **The Complete Guide to LLM Routing: 5 AI Gateways**
  Kamya Shah, Medium, Feb 2026
  https://medium.com/@kamyashah2018/the-complete-guide-to-llm-routing-5-ai-gateways-transforming-production-ai-infrastructure-b5c68ee6d641
  37% of enterprises use 5+ models in production (2026).

## Documentation

- **Using GPT-5.2 — OpenAI**
  https://platform.openai.com/docs/guides/latest-model
  Official migration guide, parameter reference, reasoning effort levels.

- **GPT-5.2 Prompting Guide — OpenAI Cookbook**
  https://cookbook.openai.com/examples/gpt-5/gpt-5-2_prompting_guide
  Official best practices for prompting GPT-5.2.

- **GPT-5 New Params and Tools — OpenAI Cookbook**
  https://cookbook.openai.com/examples/gpt-5/gpt-5_new_params_and_tools
  Custom tools, freeform inputs, CFG constraints, allowed_tools.

- **Apply Patch Tool — OpenAI**
  https://platform.openai.com/docs/guides/tools-apply-patch
  Structured diff tool for code editing via Responses API.

- **apply_patch_tool_instructions.md — OpenAI Codex**
  https://github.com/openai/codex/blob/main/codex-rs/apply-patch/apply_patch_tool_instructions.md
  Full format specification: grammar, operations, context rules, examples.

- **Structured Model Outputs — OpenAI**
  https://platform.openai.com/docs/guides/structured-outputs
  response_format and JSON schema enforcement.

- **Aider Edit Formats**
  https://aider.chat/docs/more/edit-formats.html
  Documentation of whole, diff, diff-fenced, udiff, editor-diff formats.

- **Aider Code Editing Leaderboard**
  https://aider.chat/docs/leaderboards/edit.html
  Benchmark of LLMs on 133 Exercism exercises with edit format compliance.

- **Aider Polyglot Leaderboard**
  https://aider.chat/docs/leaderboards/
  225 exercises across C++, Go, Java, JavaScript, Python, Rust.

- **Codex Prompting Guide — OpenAI Cookbook**
  https://cookbook.openai.com/examples/gpt-5/gpt-5-1-codex-max_prompting_guide
  Recommended prompting patterns for Codex CLI.
