# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [ValidateSet('all', 'lifecycle', 'audio', 'ownership', 'timer', 'cleanup')]
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
foreach ($testName in $(if ($Test -eq 'all') { @('lifecycle', 'audio', 'ownership', 'timer', 'cleanup') } else { @($Test) })) {
    if ($testName -eq 'cleanup') {
        # Compile the real cleanup functions against host DDI stubs, not a copy.
        & python (Join-Path $PSScriptRoot 'tests\extract-cleanup.py') `
            (Join-Path $PSScriptRoot 'src\Device.cpp') (Join-Path $buildRoot 'cleanup-production.inc')
        if ($LASTEXITCODE -ne 0) { throw "Cleanup source extraction failed ($LASTEXITCODE)." }
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
