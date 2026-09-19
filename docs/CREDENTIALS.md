# Credential storage

Codex Unified stores provider **references**, not provider secrets, in route or
database state.

## API keys

API-key routes use references such as:

```text
keychain://openrouter
```

The default secure-store service name is:

```text
codex-unified
```

The account component (`openrouter` above) identifies the OS credential entry.

The native secure store is supplied by the Rust `keyring` crate:

- macOS: Keychain Services;
- Windows: Windows Credential Manager;
- Unix-like desktop systems: Secret Service.

The daemon resolves the secret only when a provider request starts. It does not
write the resolved secret to SQLite, route JSON, diagnostics, or normal logs.

## OAuth

OAuth-owned clients are intentionally a separate credential-reference family,
for example:

```text
oauth://xai
```

The keychain resolver refuses those references rather than copying OAuth tokens
into Codex Unified. Provider-specific OAuth adapters should use the owning
official client/session where possible.

## CI

CI does not access the OS credential store and does not require real provider
secrets. Provider network tests use local mock servers with test-only resolvers.
