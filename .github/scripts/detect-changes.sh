#!/usr/bin/env bash
# Map a pull-request diff to the CI surfaces that must run for it.
#
# Surfaces (consumed by .github/workflows/runtime-foundation.yml):
#   pkgs       workspace crates to check/test, including every downstream
#              dependent (dependents must re-test when a dependency moves)
#   lint_pkgs  crates whose own code changed (cargo clippy targets)
#   miri_pkgs  pkgs minus moiren-app; gates the miri_suites matrix entries
#   rtcheck    the standalone rt-check harness (path-depends on core/engine/audio)
#   vad        the C++ VAD driver host suites
#   any        true when any surface above applies; gates the test job
#
# Unknown paths conservatively select the full suite, as do root manifest,
# lockfile, toolchain and workflow changes. Documentation, assets and
# audio-hack-lab match current CI coverage: nothing runs for them.
#
# Usage:
#   detect-changes.sh --base <rev> --output <file>   # diff merge-base(rev, HEAD)..HEAD
#   detect-changes.sh --files <list> --output <file> # list = newline-separated paths
# key=value lines are appended to --output ($GITHUB_OUTPUT format); a short
# human summary goes to stderr. --output defaults to stdout.
set -euo pipefail

base=""
files=""
output="/dev/stdout"
while [ $# -gt 0 ]; do
  case "$1" in
    --base) base="$2"; shift 2 ;;
    --files) files="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    *) echo "detect-changes: unknown argument: $1" >&2; exit 2 ;;
  esac
done
if [ -n "$base" ] && [ -n "$files" ]; then
  echo "detect-changes: --base and --files are mutually exclusive" >&2; exit 2
fi
if [ -z "$base" ] && [ -z "$files" ]; then
  echo "detect-changes: one of --base or --files is required" >&2; exit 2
fi

list=$(mktemp)
trap 'rm -f "$list"' EXIT
if [ -n "$base" ]; then
  git diff --name-only -M "${base}...HEAD" > "$list"
else
  cp "$files" "$list"
fi

# Canonical workspace order; the dependency closure below walks it so every
# crate is resolved before its dependents.
ORDER=(moiren-core moiren-signal moiren-engine moiren-windows-audio moiren-app)
declare -A direct=(
  [moiren-core]=false
  [moiren-signal]=false
  [moiren-engine]=false
  [moiren-windows-audio]=false
  [moiren-app]=false
)
full=false
vad=false
rtcheck=false

while IFS= read -r f; do
  [ -n "$f" ] || continue
  case "$f" in
    # Crate directories must match before the inert *.md pattern so in-crate
    # docs keep testing the crate they document.
    crates/moiren-core/*) direct[moiren-core]=true ;;
    crates/moiren-signal/*) direct[moiren-signal]=true ;;
    crates/moiren-engine/*) direct[moiren-engine]=true ;;
    crates/moiren-windows-audio/*) direct[moiren-windows-audio]=true ;;
    crates/moiren-app/*) direct[moiren-app]=true ;;
    # The C++ driver shares the VAD ABI contract with moiren-windows-audio,
    # so driver edits also run its Rust-side contract tests.
    driver/vad/*) vad=true; direct[moiren-windows-audio]=true ;;
    tools/rt-check/*) rtcheck=true ;;
    # Root manifest, lockfile, toolchain pin and workflow changes re-run everything.
    Cargo.toml|Cargo.lock|rust-toolchain.toml|rust-toolchain|.github/workflows/*) full=true ;;
    # Documentation and non-build assets keep today's CI coverage: nothing.
    *.md|docs/*|assets/*|tools/audio-hack-lab/*|.gitignore|.gitattributes|LICENSE*|PROVENANCE*) ;;
    # Anything unrecognized is treated as build-relevant: better a full run
    # than a silently skipped one.
    *) full=true ;;
  esac
done < "$list"

if $full; then
  for p in "${ORDER[@]}"; do direct[$p]=true; done
  vad=true
  rtcheck=true
fi

# true when any argument is the string "true"
any_true() {
  local v
  for v in "$@"; do
    [ "$v" = true ] && return 0
  done
  return 1
}

# Transitive dependents in dependency order: engine pulls core+signal,
# windows-audio pulls engine, app pulls windows-audio (which transitively
# covers its core/signal/engine dependencies).
declare -A closure
closure[moiren-core]=${direct[moiren-core]}
closure[moiren-signal]=${direct[moiren-signal]}
if any_true "${direct[moiren-engine]}" "${closure[moiren-core]}" "${closure[moiren-signal]}"; then
  closure[moiren-engine]=true; else closure[moiren-engine]=false; fi
if any_true "${direct[moiren-windows-audio]}" "${closure[moiren-engine]}"; then
  closure[moiren-windows-audio]=true; else closure[moiren-windows-audio]=false; fi
if any_true "${direct[moiren-app]}" "${closure[moiren-windows-audio]}"; then
  closure[moiren-app]=true; else closure[moiren-app]=false; fi
# rt-check path-depends on core, engine and windows-audio; closure of
# windows-audio transitively covers the other two.
if any_true "$rtcheck" "${closure[moiren-windows-audio]}"; then
  rtcheck=true; else rtcheck=false; fi

pkgs=()
lint_pkgs=()
miri_pkgs=()
for p in "${ORDER[@]}"; do
  if [ "${direct[$p]}" = true ]; then lint_pkgs+=("$p"); fi
  if [ "${closure[$p]}" = true ]; then
    pkgs+=("$p")
    if [ "$p" != moiren-app ]; then miri_pkgs+=("$p"); fi
  fi
done

any=false
if [ "${#pkgs[@]}" -gt 0 ] || [ "$rtcheck" = true ]; then any=true; fi

join() { local IFS=' '; echo "$*"; }
{
  echo "pkgs=$(join "${pkgs[@]}")"
  echo "lint_pkgs=$(join "${lint_pkgs[@]}")"
  echo "miri_pkgs=$(join "${miri_pkgs[@]}")"
  echo "rtcheck=$rtcheck"
  echo "vad=$vad"
  echo "any=$any"
} >> "$output"

echo "detect-changes: full=$full pkgs=[$(join "${pkgs[@]}")] lint=[$(join "${lint_pkgs[@]}")] miri=[$(join "${miri_pkgs[@]}")] rtcheck=$rtcheck vad=$vad any=$any" >&2
