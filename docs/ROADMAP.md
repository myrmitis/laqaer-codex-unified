# Roadmap

## Phase 0 — foundation

- Rust workspace and canonical protocols.
- Local daemon health/control plane.
- SQLite migration framework.
- Structured privacy-safe diagnostics.
- Transactional Codex config/catalog publisher.
- Browser-worker RPC contract.
- Golden fixtures derived from existing Router/Web behavior.

## Phase 1 — routing parity

- Native/OpenAI Responses pass-through.
- OpenAI-compatible Responses.
- OpenRouter.
- xAI/Grok.
- Credential references.
- Exact SSE terminal semantics.
- Responses WebSocket support.
- Picker import from existing Router.

These three routes—native, OpenRouter/Grok, and Web—exercise the main abstractions
before porting dozens of providers.

## Phase 2 — ChatGPT Web

- Dedicated browser profile.
- Explicit sign-in and proactive session-health state.
- Instant/Medium/High/Extra High/Pro as distinct routes.
- Typed auth/security/DOM/model errors.
- Continuation.
- Cancellation.
- Browser-only text turns.
- Image input.
- No retry storm after ambiguous submission.

## Phase 3 — local tool loop

- Strict browser tool-call envelope.
- Codex function-call conversion.
- Local sandbox/approval remains authoritative.
- Multi-round tool results.
- Cancellation and compaction.

## Phase 4 — provider parity

- Anthropic Messages.
- Kimi/Moonshot.
- DeepSeek.
- Z.ai.
- Qwen.
- MiniMax.
- ClinePass.
- Provider/model-specific schema middleware.
- External agent bridges where legally and technically appropriate.

## Phase 5 — desktop product

- One installer.
- One Control Center.
- Service supervisor.
- Browser session repair.
- Provider configuration.
- Model visibility.
- Diagnostics export.
- One-command migration and rollback.

## Acceptance gate before cutover

The old systems remain installed and untouched until the new system passes:

- native Codex model;
- Grok/OpenRouter;
- ChatGPT Web Instant;
- ChatGPT Web Pro when account-available;
- follow-up continuation;
- cancellation;
- one local tool round;
- stale browser session returns `web_auth_required`, not 502;
- security challenge is typed;
- exact rollback restores the prior Codex configuration.
