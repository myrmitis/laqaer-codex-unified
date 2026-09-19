# Architecture

## 1. Problems this system is explicitly designed to eliminate

The current combined setup exposed architectural failure modes:

1. Router state, Web Pilot state, Codex config, picker state, and browser state could disagree.
2. Generic provider normalization could delete native Codex metadata before a downstream route saw it.
3. A browser-backed model was routed through a generic HTTP-provider abstraction.
4. Browser authentication/security challenges were discovered after a turn started.
5. Retry storms obscured one browser failure behind many identical attempts.
6. Picker model identity and reasoning effort were conflated.
7. Control UI and daemon protocol could drift.
8. Generated JSON acted as both state and output.

## 2. Process model

### `codex-unifiedd` — Rust daemon

Owns:

- Codex ingress;
- local capability authentication;
- canonical turn parsing;
- route selection;
- provider registry;
- model catalog generation;
- request/response normalization;
- canonical output events;
- retry/circuit-breaker policy;
- SQLite state;
- credential references;
- diagnostics;
- browser-worker RPC.

### `browser-worker` — TypeScript sidecar

Owns only browser-specific concerns:

- isolated persistent ChatGPT profile;
- explicit sign-in UI;
- session-health state machine;
- Temporary Chat surfaces;
- model/mode selection;
- DOM interaction;
- browser continuation state;
- browser diagnostics.

It does not own the Codex catalog, Router configuration, API-provider credentials,
or retry policy.

### Desktop shell

The first desktop shell can remain Electron because the browser host already needs
Chromium primitives. It supervises the Rust daemon and browser worker using one
versioned control protocol.

## 3. Canonical turn envelope

Every inbound request is decomposed immediately:

```text
TurnEnvelope
├── trace_id
├── identity
│   ├── thread_id
│   ├── turn_id
│   ├── parent_thread_id
│   └── request_kind
├── client_metadata
├── previous_response_id
├── prompt_cache_key
├── input
├── tools
└── raw_request
```

Provider adapters receive the envelope. They may create provider-specific payloads,
but cannot mutate or erase the canonical identity.

This structurally prevents the metadata-loss class of bug.

## 4. Canonical output event stream

Providers emit internal typed events, not raw SSE strings:

- `ResponseCreated`
- `OutputItemAdded`
- `TextDelta`
- `FunctionCall`
- `FunctionCallArgumentsDelta`
- `OutputItemDone`
- `ResponseCompleted`
- `ResponseFailed`
- `ResponseIncomplete`

Only the Codex edge serializes these into Responses SSE or WebSocket frames.

EOF is never success. A request is successful only after a terminal success event.

## 5. Provider contract

Every provider declares:

- wire protocol;
- metadata policy;
- continuation policy;
- tool policy;
- retry policy;
- auth policy;
- request transform;
- response parser.

There is no global "strip unknown fields" middleware.

Provider/model-specific compatibility workarounds are attached only to the route
that demonstrated the requirement.

## 6. ChatGPT Web is first-class

Web models bypass generic API translation.

Canonical Web routes:

- `chatgpt-web/instant`
- `chatgpt-web/medium`
- `chatgpt-web/high`
- `chatgpt-web/extra-high`
- `chatgpt-web/pro`

Each route has one fixed browser mode. Pro is not another generic reasoning-effort value.

### Session state machine

```text
LoggedOut
   |
   v
Authenticating
   |
   v
Healthy <------------------+
   |                       |
   v                       |
Challenged ----recovery----+
   |
   v
NeedsReauth
```

A model turn may start only while `Healthy`.

If ChatGPT redirects to authentication during a turn:

- stop the physical browser operation;
- mark the session `NeedsReauth`;
- return typed `web_auth_required`;
- do not automatically retry;
- surface one actionable control-center message.

A Cloudflare/security challenge becomes `web_security_challenge`, not anonymous 502.

## 7. Local tools without a public MCP tunnel

The default design keeps Codex as the tool authority.

The Web model may emit a constrained structured tool-call envelope. The daemon
validates it and converts only allowed calls into canonical Codex function calls.
Codex applies its normal sandbox and approval policy, then the tool result is fed
into the next browser round.

Default implications:

- filesystem access stays local;
- terminal access stays local;
- approvals stay in Codex;
- no public tunnel;
- no browser-side direct command capability.

Full MCP can exist later as an optional adapter, not the default path.

## 8. Retry semantics

### API providers

Retry only before any downstream response item has been accepted, and only for
contract-declared retryable failures.

### Browser provider

Automatic retry is allowed only before Send activation.

Never replay automatically after:

- Send activation;
- provider acceptance;
- assistant output;
- emitted tool call.

Repeated Codex retries are deduplicated by `(thread_id, turn_id)`.

## 9. State

SQLite/WAL is the source of truth.

Initial tables:

- `providers`
- `models`
- `picker_selection`
- `capability_snapshots`
- `credential_refs`
- `browser_sessions`
- `turn_leases`
- `continuations`
- `usage_events`
- `schema_migrations`

Generated `merged-models.json` is disposable output.

Secrets remain outside SQLite:

- API keys in OS credential storage;
- official CLI OAuth remains owned by the official CLI;
- ChatGPT cookies remain in the dedicated browser profile.

## 10. Codex integration

One marked configuration block points Codex at Codex Unified.

Installation:

1. snapshot the current Codex config;
2. refuse to overwrite unknown user-owned provider/catalog settings;
3. write one managed block atomically;
4. publish one generated catalog atomically;
5. verify the exact running daemon generation;
6. instruct the user to restart Codex.

No other router source file is patched.

## 11. Observability

Every request gets one trace ID.

Privacy-safe diagnostics include:

- model;
- provider;
- stage;
- duration;
- HTTP status;
- terminal event;
- typed failure code;
- retry decision;
- browser session transition.

They exclude:

- prompts;
- cookies;
- bearer tokens;
- capability URLs;
- account IDs;
- raw browser storage.

## 12. Versioning

One product version ships:

- daemon protocol;
- browser RPC;
- DB schema;
- catalog schema;
- control protocol.

The desktop shell refuses mismatched components.
