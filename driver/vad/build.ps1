# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [switch]$Analyze,
    [ValidateSet('0', '1')]
    [string]$PlaybackCopyDiagnostic = '0'
)
$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
$buildRoot = Join-Path $projectRoot 'build'
if ($PlaybackCopyDiagnostic -eq '1') {
    $buildRoot = Join-Path $buildRoot 'playback-copy-1'
}
New-Item -ItemType Directory -Path $buildRoot -Force | Out-Null

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$msbuild = & $vswhere -latest -products '*' -requires Microsoft.Component.MSBuild `
    -find 'MSBuild\Current\Bin\amd64\MSBuild.exe' | Select-Object -First 1
if (-not $msbuild) {
    throw 'MSBuild not found. Install Visual C++ and the Windows Driver Kit.'
}
# Tool scratch output stays inside this new driver tree as well.
$previousTemp = $env:TEMP
$previousTmp = $env:TMP
$previousArchitecture = $env:PROCESSOR_ARCHITECTURE
try {
    # WDK host-tool selection falls back incorrectly when this environment
    # variable is absent in the agent shell. This process only builds x64.
    $env:PROCESSOR_ARCHITECTURE = 'AMD64'
    $env:TEMP = Join-Path $buildRoot 'temp'
    $env:TMP = $env:TEMP
    New-Item -ItemType Directory -Path $env:TEMP -Force | Out-Null
    & $msbuild (Join-Path $projectRoot 'MoirenVad.vcxproj') /m:1 /t:Build `
        /p:Configuration=Debug /p:Platform=x64 /p:PreferredToolArchitecture=x64 `
        /p:SignMode=Off /p:Inf2CatUseLocalTime=true `
        "/p:MoirenVadPlaybackCopyDiagnostic=$PlaybackCopyDiagnostic" `
        "/p:RunCodeAnalysis=$($Analyze.IsPresent.ToString().ToLowerInvariant())" `
        /verbosity:minimal /nologo `
        "/flp:logfile=$buildRoot\build.log;verbosity=normal"
    if ($LASTEXITCODE -ne 0) {
        throw "WDK build failed ($LASTEXITCODE); see build/build.log."
    }
    # Also keep an explicit native INF-verifier result beside the build log.
    $infverif = "${env:ProgramFiles(x86)}\Windows Kits\10\Tools\10.0.28000.0\x64\InfVerif.exe"
    $package = Join-Path $buildRoot 'bin\x64\Debug\MoirenVad'
    & $infverif /w /v (Join-Path $package 'MoirenVad.inf') 2>&1 |
        Tee-Object -FilePath (Join-Path $buildRoot 'infverif.log')
    if ($LASTEXITCODE -ne 0) {
        throw "Native INF validation failed ($LASTEXITCODE)."
    }
}
finally {
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
    $env:PROCESSOR_ARCHITECTURE = $previousArchitecture
}
