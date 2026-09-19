# Codex Unified

A clean-room successor to the current Codex Router + ChatGPT Web integration.

Codex Unified is one locally owned system for:

- Codex model catalog publication;
- provider routing;
- Responses SSE/WebSocket transport;
- provider credentials and capability state;
- ChatGPT Web browser routing;
- continuation, retry, cancellation and diagnostics;
- migration and rollback.

## Architecture

The core control plane is Rust. Browser automation stays TypeScript because
Electron/Chromium/Playwright are the correct tools for that layer.

```text
Codex Desktop / CLI
        |
        v
+-----------------------------+
| codex-unifiedd (Rust)       |
| ingress + auth              |
| canonical turn envelope     |
| catalog + state             |
| route selection             |
| provider adapters           |
| canonical event stream      |
| retry/circuit breaker       |
| safe diagnostics            |
+-------------+---------------+
              |
       +------+---------------------------+
       |                                  |
       v                                  v
 API/OAuth providers             local versioned RPC
                                  +----------------------+
                                  | browser-worker (TS)  |
                                  | Electron/Playwright  |
                                  | ChatGPT profile      |
                                  +----------------------+
```

The design intentionally removes the fragile chain of a generic Router provider,
bridge, Web proxy and patched Router source. ChatGPT Web is a first-class
provider.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
[docs/ROADMAP.md](docs/ROADMAP.md).

## Status

Foundation scaffold. Do not migrate a working Codex install to this repository
yet. The old systems remain the reference/rollback path until the acceptance
matrix in the roadmap is green.
