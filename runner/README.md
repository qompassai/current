# Current — v1 skeleton

A small Rust check runner for primo, replacing the GitHub
actions/runner mirror (preserved untouched on the local branch
`mirror/actions-runner-2.323.0`). Spec: `~/workspace/current-rebuild/SPEC.md`
in the estate workspace; the governing summary is in this repo's
`.ai/memory/inbox/2026-10-16-current-v1-skeleton.md`.

## What is real in the skeleton

- `current run <repo> [check] [--results <path>]` — resolves the repo
  (path, or bare name under `$CURRENT_REPO_ROOTS`, `~/.GH/Qompass`,
  `~/workspace/repos`), loads its `.current.toml`, and runs the
  selected checks **one at a time, in manifest order**.
- Manifest parsing with explicit contracts: unknown fields rejected,
  bounded manifest size / check count / argv size / timeout range,
  unique check names, relative workdirs only.
- Executor: direct argv execution (no shell), workdir must resolve
  inside the repo root, stdout/stderr drained on separate threads
  with a 1 MiB retention cap per stream (draining continues past the
  cap; truncation is recorded), hard timeout that kills and reaps
  the child, exit-code propagation.
- Results log: one JSON object per check appended to
  `$CURRENT_RESULTS_LOG` or `~/.local/state/current/results.jsonl`,
  with verdict, timing, byte totals, truncation flag, and a bounded
  tail of each stream. Nothing is sent anywhere; there is no
  telemetry path in the code.
- Exit codes: 0 = all selected checks passed, 1 = a check failed or
  timed out, 2 = usage/manifest error.

## What is stubbed (documented TODOs, not silent)

- `current poll <repo>` — the git-fetch polling trigger. Currently
  refuses with exit 2 and a TODO message. Planned: `git fetch` on
  primo's clone, run checks when the tracked branch advances.
- `current sweep` — the nightly full sweep (to be driven by a
  systemd timer). Currently refuses the same way. Planned: iterate
  the known repo roots and run every manifest.

## Manifest format

```toml
# .current.toml in a repo root
[[checks]]
name = "flake-qompass-25_05"
argv = ["nix", "build", ".#checks.x86_64-linux.x86_64-linux.qompass-25_05"]
workdir = "."
timeout_secs = 3600
env = { }
```

See `examples/ghost.current.toml` for the inaugural Ghost job.

## Gates

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```
