#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
build_dir="$script_dir/build/tests-portable"
mkdir -p -- "$build_dir"

# These suites exercise pure helpers only, without WDK or device access.
status=0
for suite in lifecycle audio ownership timer; do
    printf '\n=== VAD portable host suite: %s ===\n' "$suite"
    if ! c++ -std=c++17 -Wall -Wextra -Werror -pedantic \
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
