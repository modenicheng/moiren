# SPDX-License-Identifier: MIT
# Closed filesystem fixtures only. Never opens audio or launches an application.
[CmdletBinding()]
param([string]$Helper)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
if (-not $Helper) { $Helper = Join-Path $PSScriptRoot '..\native-report-sections.ps1' }
$tokens = $null; $parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($Helper, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw 'Helper does not parse' }
foreach ($definition in @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $true))) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('moiren-report-sections-' + [guid]::NewGuid().ToString('N'))
$null = [IO.Directory]::CreateDirectory($root)
$script:checks = 0
function Assert([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw ('ASSERTION: ' + $Message) }
    $script:checks++
}
function Refuses([scriptblock]$Action, [string]$Message) {
    $refused = $false
    try { $null = & $Action } catch { $refused = $true }
    Assert $refused $Message
}
try {
    $reportPath = Join-Path $root 'report.json'
    $engine = [ordered]@{host='Closed'; owner_generation=7; startup_trace=@{samples=@(('x' * 1000000), ('y' * 1000000))}}
    $collection = [ordered]@{quality_passed=$false; cleanup_complete=$true; failure='original native failure'; graph=@{engine=$engine}; phases=@()}
    $report = [ordered]@{native=@{engine=$engine; backend_diagnostics=@{failure='original backend failure'; native=@{streams=@(@{kind='virtual_playback'; virtual_budget=@{target_frames=1280}})}}}; collection=$collection}
    [IO.File]::WriteAllText($reportPath, ($report | ConvertTo-Json -Depth 16), [Text.UTF8Encoding]::new($false))
    $originalHash = (Get-FileHash -LiteralPath $reportPath).Hash
    $parsed = [IO.File]::ReadAllText($reportPath) | ConvertFrom-Json
    $pin = New-MoirenReportPin -ReportPath $reportPath -RelativePath 'native-run/report.json'
    $projection = New-MoirenNativeAuditSections -Report $parsed -ReportPin $pin
    $auditPath = Join-Path $root 'audit.json'
    Write-MoirenBoundedAudit -Path $auditPath -Value $projection
    Assert ((Get-Item -LiteralPath $auditPath).Length -lt 16384) 'Large startup history must not be recursively duplicated in audit'
    $actual = [IO.File]::ReadAllText($auditPath) | ConvertFrom-Json
    Assert ($actual.native_engine.json_pointer -ceq '/native/engine') 'Engine reference points at the exact original section'
    Assert ($actual.collection.graph.json_pointer -ceq '/collection/graph') 'Graph reference points at the exact original section'
    Assert ($actual.native_engine.report.sha256 -ceq $originalHash -and $actual.native_engine.report.relative_path -ceq 'native-run/report.json' -and $actual.native_engine.report.bytes -eq (Get-Item -LiteralPath $reportPath).Length) 'Reference pins complete raw bytes without transforming them'
    Assert ($actual.native_engine.scope -ceq 'file_backed_json_section_not_validated') 'A reference never claims successful validation'
    Assert ($actual.collection.quality_passed -eq $false -and $actual.collection.failure -ceq 'original native failure' -and $actual.collection.cleanup_complete -eq $true) 'Quality refusal and operational cleanup remain independent'
    Assert ($actual.native_backend_diagnostics.failure -ceq 'original backend failure' -and $actual.native_backend_diagnostics.native.streams[0].virtual_budget.target_frames -eq 1280) 'Published backend budget and failure remain literal'
    Assert ($null -ne $parsed.native.engine.startup_trace -and (Get-FileHash -LiteralPath $reportPath).Hash -ceq $originalHash) 'Projection does not mutate parsed or original report'
    Refuses { Write-MoirenBoundedAudit -Path $auditPath -Value @{replacement=$true} } 'Existing evidence is never overwritten'
    Assert (([IO.File]::ReadAllText($auditPath) | ConvertFrom-Json).collection.failure -ceq 'original native failure') 'Original audit survives refused overwrite'
    $tooLarge = Join-Path $root 'oversize.json'
    Refuses { Write-MoirenBoundedAudit -Path $tooLarge -Value @{unexpected=('z' * 1048576)} } 'Audit exceeding 1 MiB refuses rather than raising cap or truncating'
    Assert (-not (Test-Path -LiteralPath $tooLarge)) 'Oversized serialization creates no partial audit'
    $deepPath = Join-Path $root 'too-deep.json'
    $deep = [pscustomobject]@{leaf='must not turn into a string'}
    for ($level = 0; $level -lt 34; $level++) { $deep = [pscustomobject]@{child=$deep} }
    Refuses { Write-MoirenBoundedAudit -Path $deepPath -Value @{native_backend_diagnostics=$deep} } 'Deep objects refuse on PS5 instead of silent stringification'
    Assert (-not (Test-Path -LiteralPath $deepPath)) 'Depth refusal creates no partial audit'
    $empty = New-MoirenNativeAuditSections -Report ([pscustomobject]@{native=$null; collection=$null}) -ReportPin $pin
    Assert ($null -eq $empty.native_engine -and $null -eq $empty.collection -and $null -eq $empty.native_backend_diagnostics) 'Missing sections remain unknown instead of zero or synthetic evidence'
    foreach ($badPath in @('../report.json', 'native-run/../report.json', 'D:\report.json', '/report.json', 'native-run\report.json')) {
        Refuses { New-MoirenReportPin -ReportPath $reportPath -RelativePath $badPath } 'Noncanonical relative reference refuses'
    }
    $largePath = Join-Path $root 'large.json'
    $stream = [IO.File]::Open($largePath, 'CreateNew', 'Write', 'None')
    try { $stream.SetLength(33554433) } finally { $stream.Dispose() }
    Refuses { New-MoirenReportPin -ReportPath $largePath -RelativePath 'native-run/report.json' } 'Original 32 MiB report cap stays unchanged'
    Write-Output ('PASS ' + $script:checks + ' closed-report checks; no native audio/UI')
} finally {
    # Only the fresh GUID directory created by this test is removed.
    Remove-Item -LiteralPath $root -Recurse -Force
}
