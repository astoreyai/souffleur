#!/usr/bin/env bash
# Local CI gate — the same checks the project enforces, run on your own machine.
# No GitHub Actions, no billing: this is the source of truth for "is it green?".
#
#   Run manually:        ./scripts/ci.sh
#   Runs automatically:  on every `git push` (via .githooks/pre-push)
#   One-time setup:      git config core.hooksPath .githooks
#   Skip the hook once:  git push --no-verify
#
# Identical commands to a GitHub Actions runner, minus the hosted compute.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace --locked"
cargo test --workspace --locked

echo "OK: fmt clean, clippy clean, tests pass."
