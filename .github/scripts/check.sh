#!/usr/bin/env bash
#
# Everything that has to be true before blinc-markdoc is published.
#
# ONE COPY: ci.yml runs it on every push and pull request, release.yml runs
# it before anything is published. A second list of steps drifts, and the
# half that drifts is always the one nobody watches.
#
# Inputs:
#   $1          the blinc-markdoc checkout             default: $PWD
#   STRICT      1 to fail on clippy warnings           default: 1
set -euo pipefail

ROOT="${1:-$PWD}"
cd "$ROOT"
ROOT="$PWD"
STRICT="${STRICT:-1}"

group() { echo "::group::$*"; }
endgroup() { echo "::endgroup::"; }
die() { echo "::error::$*" >&2; exit 1; }

[ -f Cargo.toml ] || die "no Cargo.toml in $ROOT — point this at the blinc-markdoc checkout"

failed=""
step() {
  local name="$1"; shift
  group "$name"
  if "$@"; then
    endgroup
  else
    endgroup
    echo "::error::$name failed"
    failed="$failed $name"
  fi
}

step "cargo fmt" cargo fmt --check

if [ "$STRICT" = "1" ]; then
  step "cargo clippy" cargo clippy --all-targets -- -D warnings
else
  step "cargo clippy" cargo clippy --all-targets
fi

step "cargo test" cargo test --all-targets

# Doctests are not --all-targets.
step "cargo test --doc" cargo test --doc

# The release profile is what a consumer builds against, and it differs from
# debug in more than speed: an overflow that panics in debug wraps in release.
step "cargo build --release" cargo build --release

for s in .github/scripts/*.sh; do
  step "bash -n $(basename "$s")" bash -n "$s"
done

if [ -n "$failed" ]; then
  die "failed:$failed"
fi
echo "==> all checks passed"
