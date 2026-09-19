# Agent instructions

These invariants are part of the product contract.

1. Never delete or synthesize Codex turn identity.
2. Never convert EOF into `response.completed`.
3. Never replay a browser turn after ambiguous submission.
4. Never authenticate implicitly inside a live model turn.
5. Never store raw prompts in ordinary diagnostics.
6. Never copy ChatGPT cookies into the Rust daemon.
7. Never make generated catalog JSON the source of truth.
8. Never apply a provider-wide compatibility transform from model-specific evidence.
9. Every state mutation needs a tested rollback boundary.
10. ChatGPT Web Pro is a distinct route, not an alias for Extra High reasoning effort.
11. Provider credentials must remain local and scoped to that provider.
12. The browser worker must not execute local shell or filesystem operations directly.

## Test matrix

Every provider must cover:

- first turn;
- continuation;
- cancellation;
- explicit provider failure;
- malformed/early-ended terminal stream;
- tool call;
- provider auth failure;
- rate/quota failure.

The Web provider additionally covers:

- auth expiry before a turn;
- auth redirect during a turn;
- security challenge;
- DOM contract drift;
- ambiguous failure after Send activation;
- model/mode unavailable;
- Instant and Pro route selection.
