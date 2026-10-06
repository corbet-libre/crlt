#!/usr/bin/env bash
set -euo pipefail
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export RUNNER_TEMP="${RUNNER_TEMP:-$(mktemp -d)}"
export GITHUB_PATH="${GITHUB_PATH:-$RUNNER_TEMP/github_path}"
touch "$GITHUB_PATH"
# remote.sh gates on GITHUB_ACTIONS=true (ephemeral-runner guard); this job
# is the ephemeral runner, so assert it unconditionally — no skip branch.
export GITHUB_ACTIONS=true
# Lock hygiene: FD 8 (toolchain) and 9 (state) are owned by run-cached.sh.
# Close inherited copies so fixture daemons can never retain the locks
# (remote.sh also closes them at daemon launch; exact-PID cleanup unchanged).
exec 8>&- 9>&- 2>/dev/null || true

python3 .github/check-first-party.py
rustc --version
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
bash .ci/remote.sh

rustup run nightly rustc --version
NIGHTLY_RUSTC="$(rustup which --toolchain nightly rustc)"
NIGHTLY_CARGO="$(rustup which --toolchain nightly cargo)"
NIGHTLY_BIN="$(dirname "$NIGHTLY_RUSTC")"
export RUSTC="$NIGHTLY_RUSTC"
export CARGO="$NIGHTLY_CARGO"
export PATH="$NIGHTLY_BIN:$PATH"
export RUSTUP_TOOLCHAIN=nightly
cargo llvm-cov --version
cargo llvm-cov clean --workspace
cargo llvm-cov --locked --workspace --all-features --branch --no-cfg-coverage --no-cfg-coverage-nightly --ignore-filename-regex '/tests/' --no-report
bash .ci/remote.sh --coverage
cargo llvm-cov report --locked --branch --ignore-filename-regex '/tests/' --json --output-path coverage.json
cargo llvm-cov report --locked --branch --ignore-filename-regex '/tests/' --lcov --output-path coverage.lcov
cargo llvm-cov report --locked --branch --ignore-filename-regex '/tests/' --text --output-path coverage.txt
python3 .github/test-source-coverage.py
python3 .github/check-source-coverage.py coverage.lcov coverage.json coverage.txt
