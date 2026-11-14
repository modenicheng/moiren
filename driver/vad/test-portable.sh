#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
build_dir="$script_dir/build/tests-portable"
mkdir -p -- "$build_dir"

# Pure helpers and production-extracted lifecycle with DDI stubs; no device access.
status=0
for suite in lifecycle audio ownership timer cleanup removal hardware_release removal_diagnostics removal_trace interface_name terminal_policy pause_fence playback_copy playback_copy_disabled native_playback native_playback_disabled publication_reference; do
    printf '\n=== VAD portable host suite: %s ===\n' "$suite"
    suite_flags=()
    source_name="$suite"
    source_path="$script_dir/tests/$source_name.cpp"
    if [[ "$suite" == publication_reference ]]; then
        if ! python3 "$script_dir/tests/extract-publication-reference.py" \
            "$script_dir" "$build_dir" \
            2>&1 | tee "$build_dir/publication_reference.extract.log"; then
            status=1
            continue
        fi
        # Keep production extracts intact. Only replace the fixture's SDK DDI
        # types and wide-libc calls, whose wchar_t ABI differs on Linux.
        if ! python3 - "$script_dir" "$build_dir" <<'PY'
from pathlib import Path
import sys
root, output = map(Path, sys.argv[1:])
fixture_path = root / 'tests/interface_name_native.cpp'
fixture = fixture_path.read_text(encoding='utf-8')
for header in ('specstrings.h', 'guiddef.h', 'initguid.h', 'devpkey.h'):
    fixture = fixture.replace(f'#include <{header}>', '')
fixture = fixture.replace('using ULONG = unsigned long;', 'using ULONG = std::uint32_t;')
prefix = r'''
#include <cstdint>
#include <cstddef>
#include <cstring>
struct GUID { std::uint32_t Data1; std::uint16_t Data2, Data3; std::uint8_t Data4[8]; };
using DEVPROPTYPE = std::uint32_t;
struct DEVPROPKEY { GUID fmtid; std::uint32_t pid; };
constexpr DEVPROPTYPE DEVPROP_TYPE_EMPTY = 0, DEVPROP_TYPE_STRING = 0x12,
    DEVPROP_TYPE_STRING_INDIRECT = 0x19;
constexpr DEVPROPKEY DEVPKEY_DeviceInterface_FriendlyName =
    {{0x026e516e, 0xb814, 0x414b, {0x83, 0xcd, 0x85, 0x6d, 0x6f, 0xef, 0x48, 0x22}}, 2};
static bool IsEqualGUID(const GUID& left, const GUID& right)
{ return std::memcmp(&left, &right, sizeof(GUID)) == 0; }
static std::size_t HostWideLength(const wchar_t* text)
{ std::size_t count = 0; while (text[count] != L'\0') { ++count; } return count; }
static int HostWideCompareN(const wchar_t* left, const wchar_t* right, std::size_t count)
{
    for (std::size_t index = 0; index < count; ++index) {
        if (left[index] != right[index]) { return left[index] < right[index] ? -1 : 1; }
        if (left[index] == L'\0') { return 0; }
    }
    return 0;
}
static int HostWideCompare(const wchar_t* left, const wchar_t* right)
{ return HostWideCompareN(left, right, static_cast<std::size_t>(-1)); }
'''
for original, replacement in (('std::wcslen', 'HostWideLength'),
    ('std::wcsncmp', 'HostWideCompareN'), ('std::wcscmp', 'HostWideCompare')):
    fixture = fixture.replace(original, replacement)
source_path = root / 'tests/publication_reference.cpp'
source = source_path.read_text(encoding='utf-8')
source = source.replace('#include "interface_name_native.cpp"',
    prefix + f'\n#line 1 "{fixture_path.as_posix()}"\n' + fixture +
    f'\n#line 5 "{source_path.as_posix()}"\n')
source = source.replace('std::wcscmp', 'HostWideCompare')
(output / 'publication-reference-portable.cpp').write_text(source, encoding='utf-8')
PY
        then
            status=1
            continue
        fi
        suite_flags+=(-fshort-wchar -I "$script_dir/tests")
        source_path="$build_dir/publication-reference-portable.cpp"
    fi
    if [[ "$suite" == removal_trace ]]; then
        if ! python3 "$script_dir/tests/extract-removal-trace.py" \
            "$script_dir" "$build_dir" \
            2>&1 | tee "$build_dir/removal_trace.extract.log"; then
            status=1
            continue
        fi
    fi
    if [[ "$suite" == hardware_release ]]; then
        if ! python3 "$script_dir/tests/extract-hardware-release.py" \
            "$script_dir" "$build_dir" \
            2>&1 | tee "$build_dir/hardware_release.extract.log"; then
            status=1
            continue
        fi
    fi
    if [[ "$suite" == terminal_policy ]]; then
        if ! python3 "$script_dir/tests/extract-terminal-policy.py" \
            "$script_dir" "$build_dir" \
            2>&1 | tee "$build_dir/terminal_policy.extract.log"; then
            status=1
            continue
        fi
    fi
    if [[ "$suite" == native_playback || "$suite" == native_playback_disabled ]]; then
        if ! python3 "$script_dir/tests/extract-native-playback.py" \
            "$script_dir" "$build_dir" \
            2>&1 | tee "$build_dir/$suite.extract.log"; then
            status=1
            continue
        fi
        source_name=native_playback
        source_path="$script_dir/tests/$source_name.cpp"
        if [[ "$suite" == native_playback ]]; then
            suite_flags+=(-DMOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC=1)
        fi
    fi
    if [[ "$suite" == playback_copy || "$suite" == playback_copy_disabled ]]; then
        if ! python3 "$script_dir/tests/extract-playback-copy.py" \
            "$script_dir" "$build_dir" \
            2>&1 | tee "$build_dir/$suite.extract.log"; then
            status=1
            continue
        fi
        source_name=playback_copy
        source_path="$script_dir/tests/$source_name.cpp"
        if [[ "$suite" == playback_copy ]]; then
            suite_flags+=(-DMOIREN_VAD_PLAYBACK_COPY_DIAGNOSTIC=1)
        fi
    fi
    if [[ "$suite" == pause_fence ]]; then
        suite_flags+=(-pthread)
        if ! python3 "$script_dir/tests/extract-pause-fence.py" \
            "$script_dir" "$build_dir" \
            2>&1 | tee "$build_dir/pause_fence.extract.log"; then
            status=1
            continue
        fi
    fi
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
        "$source_path" -o "$build_dir/$suite" \
        2>&1 | tee "$build_dir/$suite.build.log"; then
        status=1
        continue
    fi
    if ! "$build_dir/$suite" 2>&1 | tee "$build_dir/$suite.log"; then
        status=1
    fi
done
exit "$status"
