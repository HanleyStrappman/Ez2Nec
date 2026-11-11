#!/bin/sh
# The checks to run before pushing to master. `cargo release` runs them too,
# and so does the release workflow.
set -e
cd "$(dirname "$0")"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
echo "check.sh: all checks passed"
