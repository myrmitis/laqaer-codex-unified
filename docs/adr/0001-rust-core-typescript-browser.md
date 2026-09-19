# ADR-0001: Rust core, TypeScript browser worker

Status: Accepted

## Decision

Use Rust for the daemon/control plane and TypeScript for the browser worker.

## Why

Rust is materially better for the always-on local router:

- predictable memory use;
- strong cancellation/concurrency semantics;
- native daemon binary;
- typed protocol boundaries;
- safer state handling;
- fewer runtime dependencies.

TypeScript remains materially better for browser automation because Playwright,
Electron, Chromium CDP, passkeys and DOM APIs are first-class there.

## Rejected alternatives

### All TypeScript

Fast to prototype but preserves too much of the runtime/process complexity the
rewrite is intended to remove.

### All Rust

Forces browser automation into a weaker ecosystem with little security benefit.

### Python/LiteLLM core

Useful as a compatibility prototype; not required in the final ordinary routing path.
