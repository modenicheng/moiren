# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [ValidateSet('all', 'lifecycle', 'audio', 'ownership', 'timer', 'cleanup', 'removal', 'diagnostics', 'clock_diagnostics')]
    [string]$Test = 'all'
)
$ErrorActionPreference = 'Stop'
$buildRoot = Join-Path $PSScriptRoot 'build\tests'
New-Item -ItemType Directory -Path $buildRoot -Force | Out-Null
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationPath | Select-Object -First 1
if (-not $vs) { throw 'MSVC x64 build tools not found.' }
$version = (Get-Content -LiteralPath "$vs\VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt").Trim()
$tools = Join-Path $vs "VC\Tools\MSVC\$version"
$sdk = "${env:ProgramFiles(x86)}\Windows Kits\10"
$compiler = Join-Path $tools 'bin\Hostx64\x64\cl.exe'
foreach ($testName in $(if ($Test -eq 'all') { @('lifecycle', 'audio', 'ownership', 'timer', 'cleanup', 'removal', 'diagnostics', 'clock_diagnostics') } else { @($Test) })) {
    if ($testName -eq 'clock_diagnostics' -and
        (Test-Path (Join-Path $PSScriptRoot 'src\audio\ClockDiagnostic.h'))) {
        # Reuse the brace-aware extractor; test the actual callback/epoch glue.
        @'
import importlib.util
from pathlib import Path
import sys
sys.dont_write_bytecode = True
root, output = map(Path, sys.argv[1:])
spec = importlib.util.spec_from_file_location('extract', root / 'tests/extract-cleanup.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
parts = []
for relative, functions in (
    ('src/Children.cpp', ('VadAudioRebaseDiscontinuity',)),
    ('src/audio/Streams.cpp', ('TimerDrivenTick', 'VadAudioTimer')),
    ('src/Control.cpp', ('TracePendingClockDiagnostic',)),
):
    source = root / relative
    patterns = tuple((name, r'(?:static\s+)?(?:void|VOID)\s+' + name +
        r'\s*\([^;{}]*\)\s*\{') for name in functions)
    parts.append(module.extract(source.read_text(encoding='utf-8'), source.as_posix(), patterns))
(output / 'clock-production.inc').write_text('\n'.join(parts), encoding='utf-8')
'@ | & python - $PSScriptRoot $buildRoot
        if ($LASTEXITCODE -ne 0) { throw "Clock source extraction failed ($LASTEXITCODE)." }
    }
    if ($testName -eq 'diagnostics') {
        & python (Join-Path $PSScriptRoot 'tests\extract-cleanup.py') --kind pcm `
            (Join-Path $PSScriptRoot 'src\Pcm.cpp') (Join-Path $buildRoot 'pcm-production.inc')
        if ($LASTEXITCODE -ne 0) { throw "PCM source extraction failed ($LASTEXITCODE)." }
    }
    if ($testName -eq 'cleanup') {
        # Compile the real cleanup functions against host DDI stubs, not a copy.
        & python (Join-Path $PSScriptRoot 'tests\extract-cleanup.py') `
            (Join-Path $PSScriptRoot 'src\Device.cpp') (Join-Path $buildRoot 'cleanup-production.inc')
        if ($LASTEXITCODE -ne 0) { throw "Cleanup source extraction failed ($LASTEXITCODE)." }
    }
    if ($testName -eq 'removal') {
        & python (Join-Path $PSScriptRoot 'tests\extract-cleanup.py') --kind removal `
            (Join-Path $PSScriptRoot 'src\Children.cpp') (Join-Path $buildRoot 'removal-production.inc')
        if ($LASTEXITCODE -ne 0) { throw "Removal source extraction failed ($LASTEXITCODE)." }
    }
    & $compiler /nologo /std:c++17 /W4 /WX /EHsc /MT `
        "/I$buildRoot" `
        "/I$tools\include" "/I$sdk\Include\10.0.28000.0\ucrt" `
        "/I$sdk\Include\10.0.28000.0\shared" "/I$sdk\Include\10.0.28000.0\um" `
        "/Fo$buildRoot\$testName.obj" "/Fe$buildRoot\$testName.exe" `
        (Join-Path $PSScriptRoot "tests\$testName.cpp") /link `
        "/LIBPATH:$tools\lib\x64" "/LIBPATH:$sdk\Lib\10.0.28000.0\ucrt\x64" `
        "/LIBPATH:$sdk\Lib\10.0.28000.0\um\x64"
    if ($LASTEXITCODE -ne 0) { throw "$testName host test compile failed ($LASTEXITCODE)." }
    & (Join-Path $buildRoot "$testName.exe") | Tee-Object -FilePath "$buildRoot\$testName.log"
    if ($LASTEXITCODE -ne 0) { throw "$testName tests failed ($LASTEXITCODE)." }
}
