#!/usr/bin/env bash
# QA Round-1: 4 test profiles + 4 clippy profiles (serialized; cargo target lock)
set -u
cd "E:/AI工作空间/随影ShadeBar" || exit 1
LOG="evidence/qa/round1-tests.log"
: > "$LOG"

run() {
  local label="$1"; shift
  {
    echo ""
    echo "########## $label ##########"
    echo "CMD: $*"
    echo "START: $(date '+%Y-%m-%d %H:%M:%S')"
  } >> "$LOG"
  "$@" >> "$LOG" 2>&1
  local rc=$?
  echo "EXIT_CODE=$rc" >> "$LOG"
  echo "END: $(date '+%Y-%m-%d %H:%M:%S')" >> "$LOG"
  echo "$label => EXIT_CODE=$rc"
  return 0
}

run "TEST default" cargo test --workspace
run "TEST verify" cargo test --workspace --features verify
run "TEST no-en" cargo test --workspace --features no-en
run "TEST all-features" cargo test --workspace --all-features
run "CLIPPY default" cargo clippy --workspace --all-targets -- -D warnings
run "CLIPPY verify" cargo clippy --workspace --all-targets --features verify -- -D warnings
run "CLIPPY no-en" cargo clippy --workspace --all-targets --features no-en -- -D warnings
run "CLIPPY all-features" cargo clippy --workspace --all-targets --all-features -- -D warnings
echo "ALL_DONE"
