# Security model

## Defaults

- Bind only to loopback.
- Use an unguessable local capability for Codex ingress.
- Never copy ChatGPT browser credentials into the Rust daemon.
- Never copy official-provider OAuth tokens when the owning CLI can be used.
- Browser worker receives only the minimum turn data required by its route.
- No public tunnel in the default Web tool path.
- No automatic local command approval.
- No fabricated success terminal.
- No browser replay after ambiguous submission.

## Browser authentication

Authentication is a control-plane operation, never an in-turn recovery action.

A turn that encounters an authentication redirect ends with a typed
`web_auth_required` failure and moves the session to `NeedsReauth`.

## Provider failures

Typed failures distinguish at least:

- `auth_required`
- `rate_limited`
- `quota_exhausted`
- `security_challenge`
- `model_unavailable`
- `dom_contract_changed`
- `provider_timeout`
- `transport_failed`
- `continuation_missing`
- `invalid_provider_response`

The product never intentionally reduces all of these to generic 502.

## Local tools

The default Web route does not grant the browser direct filesystem or terminal
access. Tool calls return to Codex, which remains the sandbox and approval authority.
