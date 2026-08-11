# agentix-harness — Architecture

## Purpose

`agentix-harness` is a general-purpose async agent loop library. It manages the
mechanics of running a local LLM in a tool-call loop: executing tools, feeding
results back to the model, detecting when the model is stuck, and escalating to
a cloud model when it has a specific knowledge gap it can articulate.

It contains no domain logic. It knows nothing about bug reports, codebases, or
security findings. Those concerns live in downstream crates that implement the
`Tool` trait.

---

## Design Principles

**Local-first.** The primary model is always a locally-hosted model (via Ollama).
The cloud is a tool the model can call, not the default path.

**Don't trust the model to know it's stuck.** Smaller models have poor
metacognition. The harness detects stagnation externally via content hashing and
injects an intervention message. The model doesn't decide — the harness tells it.

**Structured escalation.** `ask_cloud` requires three fields: what the model
already knows, a precise question, and why local sources are insufficient. This
forces deliberate escalation rather than lazy offloading.

**Hard budget.** A configurable ceiling on total tool calls prevents infinite
loops regardless of stagnation detection. When the budget is hit, the model is
forced to produce a final answer with whatever it has.

**Library, not framework.** Downstream crates implement `Tool` and wire up an
`AgentLoop`. There is no daemon, no config file, no subprocess. The harness is
just a library.

---

## Component Overview

```
┌─────────────────────────────────────────────────────────────┐
│                       agentix-harness                       │
│                                                             │
│  ┌─────────────┐    ┌──────────────────┐                   │
│  │  AgentLoop  │───▶│  GatewayClient   │──▶ agentix-daemon │
│  │  (agent.rs) │    │  (client.rs)     │    (HTTP gateway) │
│  └──────┬──────┘    └──────────────────┘                   │
│         │                                                   │
│         │  executes                                         │
│         ▼                                                   │
│  ┌──────────────┐   ┌──────────────────┐                   │
│  │  Tool trait  │   │StagnationDetector│                   │
│  │  (tool.rs)   │   │(stagnation.rs)   │                   │
│  └──────┬───────┘   └──────────────────┘                   │
│         │                                                   │
│  ┌──────▼───────┐   ┌──────────────────┐                   │
│  │   AskCloud   │   │ EscalationPolicy │                   │
│  │  (built-in)  │   │  (policy.rs)     │                   │
│  └──────────────┘   └──────────────────┘                   │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  downstream tools (implement Tool, live elsewhere)  │   │
│  │  e.g. FetchTicket, SearchCode, WriteReport          │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

```
agentix-daemon (separate crate)
  │
  ├── /v1/chat/completions ──▶ agentix-infer (local GGUF via LlamaCppBackend)
  │                        ──▶ Ollama (fallback for unregistered local models)
  │                        ──▶ Anthropic (claude-* prefix)
  │                        ──▶ OpenAI (gpt-*, o1-*, o3-*, o4-* prefix)
  │                        ──▶ OpenRouter (provider/model prefix)
  │
  ├── /v1/embeddings ───────▶ agentix-infer (primary; in-process GGUF embedding)
  │                        ──▶ Ollama (fallback if model not in local store)
  │
  └── /v1/models ──────────▶ agentix-infer local models + cloud model list + Ollama
```

`agentix-infer` is a pure Rust library crate (no network surface) linked in-process by
`agentix-daemon`. It manages a content-addressed model store (GGUF blobs), pluggable
backends (LlamaCppBackend for GGUF in Phase 1, CandleBackend for safetensors in Phase 2),
and an LRU context pool with VRAM budgeting. The daemon no longer requires Ollama to
serve embeddings — Ollama is an optional fallback for models not in the local store.

The harness talks to one URL — the agentix-daemon gateway. Routing to the right
backend (Ollama, Anthropic, OpenAI, OpenRouter) is the gateway's responsibility,
determined by model name prefix. The harness never holds API keys.

---

## The Loop

```
run(prompt)
     │
     ▼
build initial messages: [{role: user, content: prompt}]
     │
     ▼
┌────────────────────────────────────────────────────┐
│                     loop                           │
│                                                    │
│  budget exhausted? ──yes──▶ inject BUDGET_MESSAGE  │
│        │                    call model (no tools)  │
│        no                   return AgentOutput     │
│        │                                           │
│        ▼                                           │
│  call local model with tools                       │
│        │                                           │
│        ├── no tool_calls ──▶ return AgentOutput    │
│        │   (plain answer)                          │
│        │                                           │
│        ▼ has tool_calls                            │
│  echo assistant message into history               │
│  (OpenAI spec requires tool_calls array echoed     │
│   before tool-result messages)                     │
│        │                                           │
│        ▼                                           │
│  for each tool_call:                               │
│    - increment tool_calls_made                     │
│    - execute tool (or return error string)         │
│    - push result hash into StagnationDetector      │
│    - append tool-result message to history         │
│    - break if budget now exhausted                 │
│        │                                           │
│        ▼                                           │
│  stagnant? ──yes──▶ inject STAGNATION_MESSAGE      │
│        │            (appended as user message)     │
│        no           increment interventions        │
│        │                                           │
│        └──────────────────────────────── (repeat) │
└────────────────────────────────────────────────────┘
```

---

## Stagnation Detection

`StagnationDetector` maintains a sliding window of `u64` hashes of raw tool
result strings (using `std::collections::hash_map::DefaultHasher` — no
additional dependencies).

After each tool result is pushed, `is_stagnant()` checks whether the most
frequent hash in the window appears at least `min_matches` times:

```
window = [h(a), h(b), h(a), h(a)]   capacity=4, min_matches=3
                                     → stagnant (h(a) appears 3 times)

window = [h(a), h(b), h(a), h(b)]   capacity=4, min_matches=3
                                     → not stagnant (max frequency = 2)
```

This catches the common failure mode where a model re-runs the same search
with slightly different wording and receives the same content back each time.

**Default policy:** window=4, min_matches=3. Three identical results out of
the last four triggers intervention.

When stagnation fires, a `user`-role intervention message is appended to the
conversation. The stagnation window is *not* reset after intervention — if the
model immediately loops again, the next identical result will re-trigger on the
next tool call. This creates increasing pressure without an explicit counter.

---

## Escalation: AskCloud

`AskCloud` is the only built-in tool. It routes a question to a cloud model
via the same agentix-daemon gateway, using a model name in `provider/model`
format (e.g. `moonshotai/kimi-k2`, `anthropic/claude-sonnet-4-6`). The gateway
routes `provider/model` names to OpenRouter automatically.

The three required parameters prevent lazy escalation:

| Field | Purpose |
|---|---|
| `what_i_already_know` | Forces the model to summarise before escalating |
| `specific_question` | Constrains the cloud call to a precise, answerable question |
| `why_i_cant_answer_locally` | Prevents escalating questions the local model could answer with more effort |

The cloud model receives only these three fields — no conversation history, no
tool results, no domain context beyond what the local model chose to include in
`what_i_already_know`. This is the boundary that keeps sensitive information
local.

---

## Message History Format

The harness maintains `Vec<serde_json::Value>` as the conversation history.
Four message shapes are used:

```jsonc
// Initial prompt
{"role": "user", "content": "..."}

// Assistant response with tool calls
{"role": "assistant", "content": null, "tool_calls": [
  {"id": "call_abc", "type": "function",
   "function": {"name": "my_tool", "arguments": "{\"key\":\"val\"}"}}
]}
// NOTE: arguments is a JSON-encoded *string*, not an object — OpenAI wire format.

// Tool result (one per tool_call)
{"role": "tool", "tool_call_id": "call_abc", "content": "result string"}

// Injected intervention (stagnation or budget)
{"role": "user", "content": "You have retrieved similar information..."}
```

The assistant message with `tool_calls` must be echoed into history before any
tool-result messages. Ollama's OpenAI-compatible layer enforces this strictly.

---

## Implementing a Tool

```rust
use agentix_harness::Tool;
use async_trait::async_trait;
use anyhow::Result;

pub struct SearchCode {
    mcp_url: String,
}

#[async_trait]
impl Tool for SearchCode {
    fn name(&self) -> &str { "search_code" }

    fn description(&self) -> &str {
        "Search the indexed codebase for a symbol, pattern, or concept."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let query = args["query"].as_str().unwrap_or("");
        // ... call your MCP server, return results as a string
        Ok(format!("Results for: {query}"))
    }
}
```

Return a `String` from `call`. On error, return `Err(...)` — the harness catches
it and injects `"Tool error: {e}"` as the tool result, so the model sees the
failure and can adapt rather than the loop crashing.

---

## What Belongs Here vs. Downstream

| Concern | Here | Downstream |
|---|---|---|
| Loop control (budget, stagnation, intervention) | ✓ | |
| `ask_cloud` built-in tool | ✓ | |
| `Tool` trait definition | ✓ | |
| Gateway HTTP client | ✓ | |
| ClickUp / Linear / Jira fetching | | ✓ |
| MCP / code search tools | | ✓ |
| Domain-specific prompts | | ✓ |
| Report formatting | | ✓ |
| API key management | | ✓ (via agentix-daemon env) |

---

## Adding agentix-harness as a Dependency

```toml
# In your downstream crate's Cargo.toml

[dependencies]
agentix-harness = { git = "https://github.com/your-org/agentix", branch = "master" }
# or for local development:
agentix-harness = { path = "../agentix/agentix-harness" }
```

The harness does not re-export `async_trait` — downstream crates that implement
`Tool` must add it themselves:

```toml
async-trait = "0.1"
```
