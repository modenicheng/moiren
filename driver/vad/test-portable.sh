#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
build_dir="$script_dir/build/tests-portable"
mkdir -p -- "$build_dir"

# Pure helpers and production-extracted lifecycle with DDI stubs; no device access.
status=0
for suite in lifecycle audio ownership timer cleanup removal interface_name; do
    printf '\n=== VAD portable host suite: %s ===\n' "$suite"
    suite_flags=()
    if [[ "$suite" == interface_name ]]; then
        # Exercise the real UTF-16 wchar_t helper even on Linux. This suite
        # uses no wide libc calls, whose ABI may require 4-byte wchar_t.
        suite_flags+=(-fshort-wchar)
        if ! python3 "$script_dir/tests/extract-interface-name.py" \
            "$script_dir/src/Policy.cpp" "$build_dir/interface-name-production.inc" \
            2>&1 | tee "$build_dir/interface_name.extract.log"; then
            status=1
            continue
        fi
    fi
    if [[ "$suite" == cleanup ]]; then
        if ! python3 "$script_dir/tests/extract-cleanup.py" \
            "$script_dir/src/Device.cpp" "$build_dir/cleanup-production.inc" \
            2>&1 | tee "$build_dir/cleanup.extract.log"; then
            status=1
            continue
        fi
    fi
    if [[ "$suite" == removal ]]; then
        if ! python3 "$script_dir/tests/extract-cleanup.py" --kind removal \
            "$script_dir/src/Children.cpp" "$build_dir/removal-production.inc" \
            2>&1 | tee "$build_dir/removal.extract.log"; then
            status=1
            continue
        fi
    fi
    if ! c++ -std=c++17 -Wall -Wextra -Werror -pedantic \
        "${suite_flags[@]}" \
        -I "$build_dir" \
        "$script_dir/tests/$suite.cpp" -o "$build_dir/$suite" \
        2>&1 | tee "$build_dir/$suite.build.log"; then
        status=1
        continue
    fi
    if ! "$build_dir/$suite" 2>&1 | tee "$build_dir/$suite.log"; then
        status=1
    fi
done
exit "$status"
