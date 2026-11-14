# SPDX-License-Identifier: MIT
# Closed-file audit projection only. References retain provenance, not native
# authorization; callers must first establish owner join and stable artifacts.
function New-MoirenReportPin {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory=$true)][string]$ReportPath,
        [Parameter(Mandatory=$true)][string]$RelativePath
    )
    if ($RelativePath -cnotmatch '^[A-Za-z0-9_-][A-Za-z0-9_.-]*(/[A-Za-z0-9_-][A-Za-z0-9_.-]*)*$') {
        throw 'Require a canonical forward-slash relative report path'
    }
    $item = Get-Item -LiteralPath $ReportPath -ErrorAction Stop
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Require an ordinary report file'
    }
    $stream = [IO.File]::Open($item.FullName, 'Open', 'Read', 'Read')
    try {
        if ($stream.Length -gt 33554432) { throw 'Original report exceeds 32 MiB' }
        $length = $stream.Length
        $hasher = [Security.Cryptography.SHA256]::Create()
        try { $digest = $hasher.ComputeHash($stream) } finally { $hasher.Dispose() }
        [ordered]@{
            relative_path = $RelativePath
            bytes = $length
            sha256 = [BitConverter]::ToString($digest).Replace('-', '')
        }
    } finally { $stream.Dispose() }
}

function New-MoirenReportSectionReference($ReportPin, [string]$Pointer) {
    [ordered]@{
        scope = 'file_backed_json_section_not_validated'
        report = $ReportPin
        json_pointer = $Pointer
    }
}

function Get-MoirenReportField($Value, [string]$Name) {
    if ($null -ne $Value -and $null -ne $Value.PSObject.Properties[$Name]) { $Value.$Name }
}

function New-MoirenNativeAuditSections {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory=$true)]$Report,
        [Parameter(Mandatory=$true)]$ReportPin
    )
    $native = Get-MoirenReportField $Report 'native'
    $engine = Get-MoirenReportField $native 'engine'
    $collection = Get-MoirenReportField $Report 'collection'
    $projectedCollection = $null
    if ($null -ne $collection) {
        $projectedCollection = [ordered]@{}
        foreach ($property in $collection.PSObject.Properties) {
            # graph.engine repeats the same duration-dependent startup history.
            # Do not omit it silently or turn it into a validated summary.
            if ($property.Name -ceq 'graph' -and $null -ne $property.Value) {
                $projectedCollection[$property.Name] = New-MoirenReportSectionReference $ReportPin '/collection/graph'
            } else { $projectedCollection[$property.Name] = $property.Value }
        }
    }
    $engineReference = $null
    if ($null -ne $engine) { $engineReference = New-MoirenReportSectionReference $ReportPin '/native/engine' }
    [ordered]@{
        native_engine = $engineReference
        collection = $projectedCollection
        native_backend_diagnostics = Get-MoirenReportField $native 'backend_diagnostics'
    }
}

function Assert-MoirenAuditDepth($Value, [int]$Depth = 0) {
    if ($null -eq $Value) { return }
    if ($Value -is [Collections.IDictionary]) {
        if ($Depth -gt 32) { throw 'Audit object exceeds JSON depth 32' }
        foreach ($key in $Value.Keys) { Assert-MoirenAuditDepth -Value $Value[$key] -Depth ($Depth + 1) }
    } elseif ($Value -is [Management.Automation.PSCustomObject]) {
        if ($Depth -gt 32) { throw 'Audit object exceeds JSON depth 32' }
        foreach ($property in $Value.PSObject.Properties) { Assert-MoirenAuditDepth -Value $property.Value -Depth ($Depth + 1) }
    } elseif ($Value -is [Collections.IEnumerable] -and $Value -isnot [string]) {
        if ($Depth -gt 32) { throw 'Audit array exceeds JSON depth 32' }
        foreach ($element in $Value) { Assert-MoirenAuditDepth -Value $element -Depth ($Depth + 1) }
    }
}

function Write-MoirenBoundedAudit {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory=$true)][string]$Path,
        [Parameter(Mandatory=$true)]$Value
    )
    # PS5 silently stringifies over-depth objects without a warning. Check the
    # JSON-shaped tree explicitly before serialization on either supported shell.
    Assert-MoirenAuditDepth -Value $Value
    # Serialize before CreateNew: overflow/depth errors leave no partial file.
    $json = ConvertTo-Json -InputObject $Value -Depth 32 -WarningAction Stop
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($json)
    if ($bytes.Length -gt 1048576) { throw 'Audit exceeds 1 MiB; original report remains authoritative' }
    $stream = [IO.File]::Open($Path, 'CreateNew', 'Write', 'Read')
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}
