#!/usr/bin/env bash
# Self-test for detect-changes.sh: feed synthetic file lists and assert the
# selected CI surfaces. Run: bash .github/scripts/detect-changes.test.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
detect="$here/detect-changes.sh"

passed=0
failed=0

# surfaces <newline-separated-paths> -> key=value lines on stdout
surfaces() {
  local list_file out_file
  list_file=$(mktemp)
  out_file=$(mktemp)
  printf '%s\n' "$1" > "$list_file"
  bash "$detect" --files "$list_file" --output "$out_file" 2>/dev/null
  cat "$out_file"
  rm -f "$list_file" "$out_file"
}

# value <key=value-lines> <key>
value() { printf '%s\n' "$1" | sed -n "s/^$2=//p"; }

# assert <case> <key> <expected> <actual>
assert() {
  if [ "$3" = "$4" ]; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
    printf 'FAIL %s %s: expected [%s] got [%s]\n' "$1" "$2" "$3" "$4" >&2
  fi
}

ALL_PKGS='moiren-core moiren-signal moiren-engine moiren-windows-audio moiren-app'
ALL_MIRI='moiren-core moiren-signal moiren-engine moiren-windows-audio'

# App-only change: just the app surface, no Miri, no driver, no rt-check.
out=$(surfaces 'crates/moiren-app/src/ui/window/graph/tests.rs')
assert app-only pkgs 'moiren-app' "$(value "$out" pkgs)"
assert app-only lint_pkgs 'moiren-app' "$(value "$out" lint_pkgs)"
assert app-only miri_pkgs '' "$(value "$out" miri_pkgs)"
assert app-only rtcheck false "$(value "$out" rtcheck)"
assert app-only vad false "$(value "$out" vad)"
assert app-only any true "$(value "$out" any)"

# Engine change: dependents re-test; Miri covers engine and windows-audio.
out=$(surfaces 'crates/moiren-engine/tests/host/recording.rs')
assert engine pkgs 'moiren-engine moiren-windows-audio moiren-app' "$(value "$out" pkgs)"
assert engine lint_pkgs 'moiren-engine' "$(value "$out" lint_pkgs)"
assert engine miri_pkgs 'moiren-engine moiren-windows-audio' "$(value "$out" miri_pkgs)"
assert engine rtcheck true "$(value "$out" rtcheck)"
assert engine vad false "$(value "$out" vad)"

# Core and signal fan out to their dependents. moiren-signal does not depend
# on moiren-core, so a core change never pulls the signal crate in.
out=$(surfaces 'crates/moiren-core/src/graph.rs')
assert core pkgs 'moiren-core moiren-engine moiren-windows-audio moiren-app' "$(value "$out" pkgs)"
assert core lint_pkgs 'moiren-core' "$(value "$out" lint_pkgs)"
assert core miri_pkgs 'moiren-core moiren-engine moiren-windows-audio' "$(value "$out" miri_pkgs)"
out=$(surfaces 'crates/moiren-signal/src/transport/stream.rs')
assert signal pkgs 'moiren-signal moiren-engine moiren-windows-audio moiren-app' "$(value "$out" pkgs)"
assert signal lint_pkgs 'moiren-signal' "$(value "$out" lint_pkgs)"
assert signal miri_pkgs 'moiren-signal moiren-engine moiren-windows-audio' "$(value "$out" miri_pkgs)"

# windows-audio change: itself plus app and rt-check.
out=$(surfaces 'crates/moiren-windows-audio/src/vad/provider.rs')
assert audio pkgs 'moiren-windows-audio moiren-app' "$(value "$out" pkgs)"
assert audio miri_pkgs 'moiren-windows-audio' "$(value "$out" miri_pkgs)"
assert audio rtcheck true "$(value "$out" rtcheck)"

# Driver edit: C++ host suites plus the Rust-side VAD contract tests.
out=$(surfaces 'driver/vad/src/Device.cpp')
assert driver vad true "$(value "$out" vad)"
assert driver pkgs 'moiren-windows-audio moiren-app' "$(value "$out" pkgs)"
assert driver lint_pkgs 'moiren-windows-audio' "$(value "$out" lint_pkgs)"
# Driver docs still exercise the ABI contract the tests pin against.
out=$(surfaces 'driver/vad/ABI.md')
assert driver-docs vad true "$(value "$out" vad)"

# rt-check only: harness tests without dragging the workspace in.
out=$(surfaces 'tools/rt-check/src/main.rs')
assert rtcheck pkgs '' "$(value "$out" pkgs)"
assert rtcheck lint_pkgs '' "$(value "$out" lint_pkgs)"
assert rtcheck rtcheck true "$(value "$out" rtcheck)"
assert rtcheck any true "$(value "$out" any)"

# Documentation, assets and audio-hack-lab run nothing.
out=$(surfaces 'docs/plans/x.md
README.md
AGENTS.md
assets/logos/moiren.svg
tools/audio-hack-lab/src/main.rs')
assert inert any false "$(value "$out" any)"
assert inert pkgs '' "$(value "$out" pkgs)"
assert inert vad false "$(value "$out" vad)"
assert inert rtcheck false "$(value "$out" rtcheck)"

# Root manifest, lockfile, toolchain and workflow edits run everything.
for trigger in Cargo.toml Cargo.lock rust-toolchain.toml .github/workflows/runtime-foundation.yml; do
  out=$(surfaces "$trigger")
  assert "full($trigger)" pkgs "$ALL_PKGS" "$(value "$out" pkgs)"
  assert "full($trigger)" lint_pkgs "$ALL_PKGS" "$(value "$out" lint_pkgs)"
  assert "full($trigger)" miri_pkgs "$ALL_MIRI" "$(value "$out" miri_pkgs)"
  assert "full($trigger)" rtcheck true "$(value "$out" rtcheck)"
  assert "full($trigger)" vad true "$(value "$out" vad)"
done

# An unrecognized path is conservative: full run.
out=$(surfaces 'some/new/tool.cpp')
assert unknown pkgs "$ALL_PKGS" "$(value "$out" pkgs)"
assert unknown any true "$(value "$out" any)"

# Mixed change: union of surfaces, docs add nothing.
out=$(surfaces 'crates/moiren-app/src/service/managed.rs
crates/moiren-engine/src/buffer.rs
docs/notes.md')
assert mixed pkgs 'moiren-engine moiren-windows-audio moiren-app' "$(value "$out" pkgs)"
assert mixed lint_pkgs 'moiren-engine moiren-app' "$(value "$out" lint_pkgs)"

# A rename reports old and new paths; both areas count.
out=$(surfaces 'crates/moiren-app/src/ui/old.rs
crates/moiren-engine/src/new.rs')
assert rename lint_pkgs 'moiren-engine moiren-app' "$(value "$out" lint_pkgs)"

# Docs inside a crate still test that crate.
out=$(surfaces 'crates/moiren-core/README.md')
assert crate-doc pkgs 'moiren-core moiren-engine moiren-windows-audio moiren-app' "$(value "$out" pkgs)"

printf 'detect-changes self-test: %d passed, %d failed\n' "$passed" "$failed"
[ "$failed" -eq 0 ]
