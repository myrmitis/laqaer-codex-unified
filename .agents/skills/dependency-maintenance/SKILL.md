---
name: dependency-maintenance
description: Review Cargo, browser-worker and Actions updates without bypassing repository invariants.
---

# Dependency maintenance

Read AGENTS.md, .github/dependabot.yml, the Cargo workspace, browser-worker manifest
and lockfiles, and the exact PR diff. Preserve every provider/turn-identity invariant.

Routine minor/patch groups are not a safety approval. Major upgrades remain separate
and need explicit integration acceptance. Security updates take priority; record
alerts/security-update settings as observed or unverified, never inferred from YAML.

Inspect source registries, new transitive code, build/install scripts, licenses,
Rust edition/toolchain support, Node/TypeScript compatibility, credential-library
changes and provider/protocol regressions. Run the existing CI/test matrix appropriate
to the changed surface with reproducible lockfile installs.

Never expose provider credentials or browser cookies, weaken tests, run untrusted
head code through privileged pull_request_target, or add merge/deploy authority to
fix bot CI. Isolate incompatible updates rather than broadening compatibility logic.

Return exact base/head, checks actually run, residual risks, rollback and an
accept/repair recommendation. Do not merge or deploy without separate authority.
