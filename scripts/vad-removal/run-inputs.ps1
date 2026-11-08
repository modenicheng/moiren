#requires -Version 5.1
# Read-only inputs for one diagnostic. File pins are provenance, not native
# approval. Definitions never launch a tool, discover devices or create outputs.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'trace-decode.ps1')

function Get-RemovalRunFixedPins {
    @{
        ProbeSource='26cd706f773f9bfb75e9572fc66594b5f932b918';ProbeBytes=7752704
        ProbeSha256='C8AB9DB102B91CE0AE3724D1856279678019909FB0E9F8F271C77B3453A63266'
        DriverRevision='23807f3dc482090c0dd6f0a9775cd91b8d6c1da6'
        DriverManifestSha256='DEF621D149EF43753D27B8CE2A8DFA1237E6D8B106D369931A50F07E24AC1317'
        PdbSha256='775C5418D7EFC76433B4E4708F1ADEB01FA5F6600F302BFE03B880F13CB100DE'
        DriverFiles=@(
            @{Name='moirenvad.cat';Bytes=2514;Sha256='A6171C7C474667BF4A17B999D3187A325D509BA742EF89BA7322C8384E181E20'},
            @{Name='MoirenVad.inf';Bytes=1799;Sha256='24574ED3BAE942EF7ED91601513F8F0B181272BF1F15D2067F0F6EE2014F827C'},
            @{Name='MoirenVad.sys';Bytes=152472;Sha256='0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76'})
        ScheduleSha256='4FBD145DD4CCD8D847DDE56D87B5AA023748B813E4118AB54FC2D08A6066C78D'
        SignToolSha256='C8AC2561B1E75031EE864CCA3FB2B4CD06630E3EEA42D86AB80A573BCF8B9472'
        CatalogSha256='58552AEC72650250FA0C01A309B6B885561A38F925842175C1C5B0EEEB0F081A'
        FiniteSha256='C3621DC9C881519006E01619D2C65E053C3A1F0FBFC100C0E247E3064D16651B'
    }
}

function Get-RemovalRunSourceNames {
    @('closed-export.ps1','collector-boundary.ps1','collector-worker.ps1','controller-seal.ps1','diagnostic-controller.ps1','diagnostic-lease.ps1',
        'finish-session.ps1','finite-tool.ps1','invoke-diagnostic.ps1','invoke-preflight.ps1','native-admission.ps1',
        'preflight-session.ps1','probe-boundary.ps1','readonly-preflight.ps1','run-inputs.ps1',
        'safety-audit.ps1','session-identity.ps1','target-readiness.ps1','task-boundary.ps1',
        'trace-boundary.ps1','trace-decode.ps1','trace-native.ps1','trace-query.ps1',
        'trace-readback.ps1','trace-session.ps1','worker-boundary.ps1','worker.ps1')
}

function Assert-RemovalRunInteger($Value,[long]$Minimum,[long]$Maximum) {
    if(($Value -isnot [int] -and $Value -isnot [long]) -or $Value -lt $Minimum -or $Value -gt $Maximum){
        throw 'Typed bounded JSON integer required for diagnostic inputs.'
    }
}

function Copy-RemovalRunSources($Rows,[string]$Directory) {
    Assert-FiniteToolPath $Directory $false
    if(-not(Test-Path -LiteralPath $Directory -PathType Container)){throw 'Existing production source directory required.'}
    if($Rows -isnot [array] -or $Rows.Count -lt 27 -or $Rows.Count -gt 64){throw 'Complete bounded production source membership required.'}
    $names=Get-RemovalRunSourceNames;$seen=@{};$pins=@();$total=[long]0
    foreach($row in $Rows){
        if($row.Name -isnot [string] -or $row.Name -cnotmatch '\A[A-Za-z0-9][A-Za-z0-9_.-]*\.ps1\z' -or
            $row.Name -like 'test-*' -or $seen.ContainsKey($row.Name) -or
            $row.Path -cne (Join-Path $Directory $row.Name)){throw 'Source name/path missing, duplicate or outside exact production directory.'}
        $seen[$row.Name]=$true;$pin=Copy-RemovalDecodePin $row 1048576
        $pins+=,[pscustomobject]@{Name=$row.Name;Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256};$total+=$pin.Bytes
    }
    foreach($name in $names){if(-not$seen.ContainsKey($name)){throw ('Missing required production dependency: '+$name)}}
    if($total -gt 16777216){throw 'Production source aggregate cap exceeded.'}
    # New production libraries must be included, not silently ignored because
    # they were absent when this list was first written. Tests/logs are excluded.
    $actual=@(Get-ChildItem -LiteralPath $Directory -Force -Filter '*.ps1' -ErrorAction Stop |
        Where-Object{$_.Name -notlike 'test-*'})
    if($actual.Count -ne $pins.Count){throw 'Actual production source membership differs from manifest.'}
    foreach($file in $actual){
        if($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            -not$seen.ContainsKey($file.Name)){throw 'Unpinned or nonregular production dependency.'}
    }
    return ,$pins
}

function Assert-RemovalRunRelease($Probe,$Fixed) {
    if($Probe.SourceRevision -cne $Fixed.ProbeSource -or $Probe.File.Bytes -ne $Fixed.ProbeBytes -or
        $Probe.File.Sha256 -ine $Fixed.ProbeSha256 -or
        [IO.Path]::GetFileName($Probe.File.Path) -cne 'e2e_latency_probe.exe'){throw 'Exact original managed release pin required.'}
    $build=(Read-RemovalDecodeFile $Probe.BuildManifest.Path 1048576 -Text).Text|ConvertFrom-Json
    Assert-RemovalRunInteger $build.BuildExit 0 0
    if($build.SourceHead -isnot [string] -or $build.SourceHead -cne $Fixed.ProbeSource -or
        $build.RustSourcesMatchSourceHead -isnot [bool] -or -not$build.RustSourcesMatchSourceHead -or
        $build.Files -isnot [array] -or $build.Files.Count -lt 1 -or $build.Files.Count -gt 64){
        throw 'Actual SourceHead/Files managed release manifest required; no historical-source substitution.'
    }
    $rows=@($build.Files|Where-Object{$_.Name -ieq 'e2e_latency_probe.exe'})
    if($rows.Count -ne 1 -or $rows[0].Name -cne 'e2e_latency_probe.exe'){throw 'Unique managed release EXE row required.'}
    $row=$rows[0];Assert-RemovalRunInteger $row.Bytes 1 268435456
    if($row.Path -isnot [string] -or $row.Path -notmatch '\A[A-Za-z]:\\' -or
        [IO.Path]::GetFullPath($row.Path) -cne $row.Path -or $row.Path.Substring(2).Contains(':') -or
        [IO.Path]::GetFileName($row.Path) -cne 'e2e_latency_probe.exe' -or
        $row.Bytes -ne $Probe.File.Bytes -or $row.Sha256 -isnot [string] -or $row.Sha256 -ine $Probe.File.Sha256){
        throw 'Original build EXE bytes/hash differs; relocation cannot relabel content.'
    }
}

function Copy-RemovalRunDriver($Driver,$Fixed) {
    $package=Copy-RemovalDecodePin $Driver.PackageManifest 1048576
    $pdb=Copy-RemovalDecodePin $Driver.Pdb 16777216
    if($package.Sha256 -ine $Fixed.DriverManifestSha256 -or $pdb.Sha256 -ine $Fixed.PdbSha256 -or
        [IO.Path]::GetFileName($pdb.Path) -cne 'MoirenVad.pdb'){throw 'Frozen613 package/PDB identity differs.'}
    $build=(Read-RemovalDecodeFile $package.Path 1048576 -Text).Text|ConvertFrom-Json
    Assert-RemovalRunInteger $build.SchemaVersion 1 1
    if($build.DriverVersion -cne '3.1.1.613' -or $build.Revision -cne $Fixed.DriverRevision -or
        $build.PdbSha256 -ine $Fixed.PdbSha256 -or $build.Files -isnot [array] -or $build.Files.Count -ne 3 -or
        $Driver.Files -isnot [array] -or $Driver.Files.Count -ne 3){throw 'Original613 package schema/source/file set differs.'}
    $files=@();$directory=$null
    foreach($expected in $Fixed.DriverFiles){
        $rows=@($Driver.Files|Where-Object{$_.Name -ieq $expected.Name})
        $record=@($build.Files|Where-Object{$_.Name -ieq $expected.Name})
        if($rows.Count -ne 1 -or $record.Count -ne 1 -or $rows[0].Name -cne $expected.Name -or
            $record[0].Name -cne $expected.Name){throw 'Exact unique signed package membership required.'}
        $pin=Copy-RemovalDecodePin $rows[0] 16777216
        Assert-RemovalRunInteger $record[0].Bytes 1 16777216
        if([IO.Path]::GetFileName($pin.Path) -cne $expected.Name -or $pin.Bytes -ne $expected.Bytes -or
            $pin.Sha256 -ine $expected.Sha256 -or $record[0].Bytes -ne $pin.Bytes -or
            $record[0].Sha256 -ine $pin.Sha256){throw 'Signed613 package member differs from fixed original.'}
        $parent=[IO.Path]::GetDirectoryName($pin.Path)
        if($null -eq $directory){$directory=$parent}elseif($directory -cne $parent){throw 'Signed package files must belong to one exact directory.'}
        $files+=,[pscustomobject]@{Name=$expected.Name;Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256}
    }
    [pscustomobject]@{PackageManifest=$package;Files=$files;Pdb=$pdb}
}

function Copy-RemovalRunSymbols($Symbols,$Fixed) {
    Assert-FiniteToolPath $Symbols.Directory $false
    if(-not(Test-Path -LiteralPath $Symbols.Directory -PathType Container) -or
        $Symbols.Files -isnot [array] -or $Symbols.Files.Count -lt 1 -or $Symbols.Files.Count -gt 64){
        throw 'Existing exact flat bounded TMF membership required.'
    }
    $seen=@{};$files=@();$total=[long]0
    foreach($row in $Symbols.Files){
        $pin=Copy-RemovalDecodePin $row 1048576
        $name=[IO.Path]::GetFileName($pin.Path)
        if($seen.ContainsKey($name) -or [IO.Path]::GetDirectoryName($pin.Path) -cne $Symbols.Directory -or
            [IO.Path]::GetExtension($name) -ine '.tmf'){throw 'Duplicate/outside/non-TMF symbol member.'}
        $seen[$name]=$true;$files+=,$pin;$total+=$pin.Bytes
    }
    $actual=@(Get-ChildItem -LiteralPath $Symbols.Directory -Force -ErrorAction Stop)
    if($total -gt 16777216 -or $actual.Count -ne $files.Count){throw 'Exact TMF aggregate/membership differs.'}
    foreach($file in $actual){
        if($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            -not$seen.ContainsKey($file.Name)){throw 'Unpinned or nonregular TMF member.'}
    }
    [pscustomobject]@{Directory=$Symbols.Directory;Files=$files;DriverRevision=$Fixed.DriverRevision;PdbSha256=$Fixed.PdbSha256}
}

function Read-RemovalRunPins($Manifest,[string]$SourceDirectory) {
    $fixed=Get-RemovalRunFixedPins
    Assert-RemovalRunInteger $Manifest.SchemaVersion 1 1
    if($Manifest.SourcesRevision -isnot [string] -or $Manifest.SourcesRevision -cnotmatch '\A[0-9a-f]{40}\z'){
        throw 'Reviewed production source revision required.'
    }
    $sources=Copy-RemovalRunSources $Manifest.Sources $SourceDirectory
    $finite=@($sources|Where-Object{$_.Name-ceq'finite-tool.ps1'})
    if($finite.Count -ne 1 -or $finite[0].Sha256 -ine $fixed.FiniteSha256){throw 'Frozen finite-tool predecessor differs.'}
    $probe=[pscustomobject]@{File=(Copy-RemovalDecodePin $Manifest.Probe.File 268435456)
        BuildManifest=(Copy-RemovalDecodePin $Manifest.Probe.BuildManifest 1048576);SourceRevision=$Manifest.Probe.SourceRevision}
    Assert-RemovalRunRelease $probe $fixed
    $driver=Copy-RemovalRunDriver $Manifest.Driver $fixed
    $schedule=Copy-RemovalDecodePin $Manifest.Schedule 1048576
    if($schedule.Sha256 -ine $fixed.ScheduleSha256){throw 'Original source schedule differs; no fitting or rescheduling.'}
    $tools=@{}
    foreach($entry in @(@{Key='Tracefmt';Name='tracefmt.exe';Cap=268435456},@{Key='Python';Name='python.exe';Cap=268435456},
        @{Key='Reader';Name='vad-removal-stages.py';Cap=1048576},@{Key='SignTool';Name='signtool.exe';Cap=268435456},
        @{Key='Catalog';Name='w00_probe.exe';Cap=268435456},@{Key='BcdEdit';Name='bcdedit.exe';Cap=268435456})){
        $pin=Copy-RemovalDecodePin $Manifest.Tools.($entry.Key) $entry.Cap
        if([IO.Path]::GetFileName($pin.Path) -ine $entry.Name){throw 'Wrong exact diagnostic tool/reader file name.'}
        $tools[$entry.Key]=$pin
    }
    if($tools.SignTool.Sha256 -ine $fixed.SignToolSha256 -or $tools.Catalog.Sha256 -ine $fixed.CatalogSha256){
        throw 'Frozen readonly safety tool differs.'
    }
    [pscustomobject]@{SourcesRevision=$Manifest.SourcesRevision;Sources=$sources;Probe=$probe;Driver=$driver
        Schedule=$schedule;Tools=$tools;Symbols=(Copy-RemovalRunSymbols $Manifest.Symbols $fixed)
        DriverPackage=$driver.PackageManifest;Tracefmt=$tools.Tracefmt;Python=$tools.Python;Reader=$tools.Reader}
}

function Get-RemovalRunConfigJson($Owner) {
    ConvertTo-Json -InputObject ([ordered]@{ManifestPath=$Owner.ManifestPath;ExpectedSha256=$Owner.ExpectedSha256
        ManifestPin=$Owner.ManifestPin;SourceDirectory=$Owner.SourceDirectory;SourceRevision=$Owner.SourceRevision
        Schedule=$Owner.Schedule}) -Depth 6 -Compress
}

function New-RemovalRunInputs {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$ManifestPath,
        [Parameter(Mandatory=$true)][ValidatePattern('\A[A-Fa-f0-9]{64}\z')][string]$ExpectedSha256,
        [string]$SourceDirectory=$PSScriptRoot)
    $ErrorActionPreference='Stop'
    $file=Read-RemovalDecodeFile $ManifestPath 1048576 -Text
    if($file.Sha256 -ine $ExpectedSha256){throw 'Independent diagnostic manifest SHA256 differs; never adopt new content.'}
    $manifest=$file.Text|ConvertFrom-Json
    $pins=Read-RemovalRunPins $manifest $SourceDirectory
    $owner=[pscustomobject]@{ManifestPath=$ManifestPath;ExpectedSha256=$ExpectedSha256.ToUpperInvariant()
        ManifestPin=[pscustomobject]@{Path=$file.Path;Bytes=$file.Bytes;Sha256=$file.Sha256}
        ManifestJson=$file.Text;SourceDirectory=$SourceDirectory;SourceRevision=$pins.SourcesRevision
        Schedule=$pins.Schedule;Pins=$pins;ConfigJson=$null;PinsJson=$null}
    $owner.ConfigJson=Get-RemovalRunConfigJson $owner
    $owner.PinsJson=ConvertTo-Json -InputObject $pins -Depth 14 -Compress
    Assert-RemovalRunInputs $owner
    return $owner
}

function Assert-RemovalRunInputs {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if((Get-RemovalRunConfigJson $Owner) -cne $Owner.ConfigJson -or
        (ConvertTo-Json -InputObject $Owner.Pins -Depth 14 -Compress) -cne $Owner.PinsJson){
        throw 'Original diagnostic input configuration/pins changed.'
    }
    $file=Read-RemovalDecodeFile $Owner.ManifestPath 1048576 -Text
    if($file.Sha256 -ine $Owner.ExpectedSha256 -or $file.Bytes -ne $Owner.ManifestPin.Bytes -or
        $file.Text -cne $Owner.ManifestJson){throw 'Original immutable diagnostic manifest changed.'}
    # Reread every actual production dependency, not just the manifest or public
    # DTO. Returned pins are detached values; each use reestablishes file identity.
    $pins=Read-RemovalRunPins ($file.Text|ConvertFrom-Json) $Owner.SourceDirectory
    if((ConvertTo-Json -InputObject $pins -Depth 14 -Compress) -cne $Owner.PinsJson){
        throw 'Actual diagnostic dependency identity changed.'
    }
}

function Get-RemovalDiagnosticArguments {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[Parameter(Mandatory=$true)][string]$ArtifactDirectory)
    $ErrorActionPreference='Stop';Assert-RemovalRunInputs $Owner
    Assert-FiniteToolPath $ArtifactDirectory $false
    @('live-vad','--stable-endpoint-id','4d6f6972656e53315761766530343330',
        '--name','Moiren S1 Shared Waveform 0430','--vad-source-mode','shared',
        '--source-shared-period-frames','480','--source-schedule',$Owner.Schedule.Path,
        '--vad-input-capture-frames','480000','--output','{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}',
        '--render-mode','shared','--render-period-frames','336','--render-target-frames','512',
        '--wake-margin-frames','64','--callback-budget-frames','64','--engine-block-frames','64',
        '--artifact-dir',$ArtifactDirectory,'--seconds','8')
}

function New-RemovalDiagnosticDecodePins {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[Parameter(Mandatory=$true)]$TraceSession,
        [Parameter(Mandatory=$true)]$ManagedSession)
    $ErrorActionPreference='Stop';Assert-RemovalRunInputs $Owner
    $stop=Assert-RemovalDecodeStoppedSession $TraceSession
    $arguments=@($ManagedSession.Config.ProbeArguments)
    $indices=@(for($i=0;$i -lt $arguments.Count;$i++){if($arguments[$i] -ceq '--artifact-dir'){$i}})
    if($indices.Count -ne 1 -or $indices[0]+1 -ge $arguments.Count){throw 'Unique original artifact directory required.'}
    $artifact=$arguments[$indices[0]+1]
    $expected=@(Get-RemovalDiagnosticArguments $Owner -ArtifactDirectory $artifact)
    if($arguments.Count -ne $expected.Count){throw 'Original managed diagnostic argv differs.'}
    for($i=0;$i -lt $expected.Count;$i++){if($arguments[$i] -cne $expected[$i]){throw 'Original managed diagnostic argv differs.'}}
    $etl=Read-RemovalDecodeFile $stop.EtlPath 16777216
    $report=Read-RemovalDecodeFile (Join-Path $artifact 'report.json') 16777216
    $pins=@{SchemaVersion=1;Etl=[pscustomobject]@{Path=$etl.Path;Bytes=$etl.Bytes;Sha256=$etl.Sha256}
        ProbeReport=[pscustomobject]@{Path=$report.Path;Bytes=$report.Bytes;Sha256=$report.Sha256}
        DriverPackage=$Owner.Pins.DriverPackage;Tracefmt=$Owner.Pins.Tracefmt;Python=$Owner.Pins.Python
        Reader=$Owner.Pins.Reader;Probe=$Owner.Pins.Probe;Symbols=$Owner.Pins.Symbols}
    $check=[pscustomobject]@{TraceSession=$TraceSession;ManagedSession=$ManagedSession;Pins=$pins}
    $null=Assert-RemovalDecodeManagedReport $check
    Assert-RemovalDecodeInputs $check
    # Do not expose aliases to the immutable input owner through the returned map.
    return (ConvertTo-Json -InputObject $pins -Depth 12 -Compress|ConvertFrom-Json)
}
