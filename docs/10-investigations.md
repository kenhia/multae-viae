# Investigations: Tool Use Reliability

Open questions and observations around LLM tool calling that need deeper
investigation. These are deferred from sprint 004 (MCP Integration) to avoid
scope creep.

## Observation: Small Models Outperformed Large on Tool Use

During sprint 004 testing, `qwen3:8b` produced more consistent tool-calling
results than `qwen3-coder:30b`. This is counterintuitive — larger models
typically have better instruction following.

**Possible explanations to investigate:**

- `qwen3-coder` is fine-tuned for code generation, not agentic tool use.
  The base `qwen3` series may have better tool-calling alignment.
- The larger model may be "overthinking" — generating reasoning instead of
  emitting a tool call when appropriate.
- Prompt format differences: Ollama may template tool definitions differently
  per model family. The tool-call prompt format that works for `qwen3` may
  not be optimal for `qwen3-coder`.
- Context window effects: more tools in the prompt may push the larger model
  past a quality threshold for its quantization level.

**Action items:**

- [ ] Test `qwen3:32b` (base, not coder) for tool reliability
- [ ] Compare tool-call success rates across model sizes with a fixed test set
- [ ] Check Ollama's model-specific tool-call prompt templates
- [ ] Test with `/no_think` tag to suppress reasoning and force direct tool use

## Issue: Tool Count and Model Confusion

When many tools are available (built-in + MCP), smaller models sometimes:

- Ignore tools and answer from memory
- Call the wrong tool (e.g., `shell_exec` when `file_list` is more appropriate)
- Generate malformed JSON for tool arguments
- Call a tool correctly but ignore its output in the final response

**Mitigations already in place:**

- `SEMANTIC_OVERLAPS` list in `registry.rs` filters MCP tools that duplicate
  built-in capabilities
- `MAX_TOOL_OUTPUT_CHARS` truncation prevents context window overflow
- `default_max_turns(10)` caps tool-calling loops

**Mitigations to investigate:**

- [ ] Dynamic tool filtering: only present tools relevant to the query
- [ ] Better system preamble with explicit tool-use instructions
- [ ] Tool-capability-aware model routing (Phase 7 roadmap item)
- [ ] Retry with reformulated prompt when tool call produces malformed JSON

## Issue: No Feedback on Turn Limit

When the agent hits `default_max_turns(10)`, the user gets whatever partial
response the model last produced. There is no indication that the turn limit
was reached.

**To investigate:**

- [ ] Does Rig expose a signal when max turns are exhausted?
- [ ] Should we log a warning and/or append a notice to the response?

## Issue: Cloud vs Local for Tool-Heavy Workloads

The OpenAI provider is implemented and tested but not actively used. For
tool-heavy queries, cloud models (e.g., `gpt-4o-mini`) are significantly
more reliable.

**To investigate:**

- [ ] Automatic fallback: try local model, fall back to cloud on tool-call
  failure
- [ ] Cost tracking: log estimated token costs for cloud calls
- [ ] Hybrid routing: use local for simple queries, cloud for tool-heavy ones
  (ties into Phase 7 model routing)
