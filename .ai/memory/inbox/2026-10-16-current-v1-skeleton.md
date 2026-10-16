# Current v1 skeleton — decision and state (2026-10-16)

Decision (Matt, 2026-10-15): rebuild Current from scratch as a small
Rust local check runner; do not carry the GitHub actions/runner fork.
Full spec: `~/workspace/current-rebuild/SPEC.md` in the estate
workspace.

State after the 2026-10-16 night run:

- The actions/runner 2.323.0 mirror is preserved untouched on the
  LOCAL branch `mirror/actions-runner-2.323.0` at the former main tip
  (0cfb1bd). Not pushed; landing stays Matt's word.
- The v1 skeleton lives on local branch
  `pax/current-v1-skeleton-20261016` under `runner/`: CLI, bounded
  `.current.toml` manifest parsing, executor with hard timeout and
  capped log capture, JSONL results log.
- Real: `current run <repo> [check]`. Stubbed as documented TODOs:
  the git-fetch poll trigger and the nightly sweep.
- Inaugural job: Ghost's `qompass-25_05` VM test, manifest in
  `runner/examples/ghost.current.toml` and placed at the Ghost
  checkout root as `.current.toml`.
