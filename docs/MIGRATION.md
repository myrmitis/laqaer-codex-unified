# Migration strategy

Codex Unified uses its own home:

```text
~/.codex-unified/
```

It may inspect old Router/Web Pilot state but never mutates those directories.

## Import

`codex-unified migrate inspect` will eventually read:

- the current managed Codex provider/catalog block;
- Router provider/model selection;
- Router credential references;
- Web capability observations;
- browser-profile health metadata.

Import remains read-only until `migrate apply`.

## Side-by-side period

- The new daemon uses a different loopback port and capability.
- Old Router/Web Pilot remain installed.
- Only one managed Codex block is active at a time.
- Switching is transactional and reversible.

## Cutover

1. Snapshot existing `~/.codex/config.toml`.
2. Activate the Codex Unified managed block.
3. Publish the Unified catalog.
4. Restart Codex.
5. Run the acceptance matrix.
6. Disable old services but do not delete them during the rollback window.

## Rollback

```text
codex-unified migrate rollback
```

restores exact pre-cutover Codex config/service state.

Deletion of the old systems is a separate explicit operation.
