#requires -Version 5.1
# Closed-file analysis only. Loading definitions launches no process or ETW API.
# Tracefmt syntax: closed.etl -p exact-tmf-directory -o new-driver.txt;
# the summary is new-driver.txt.sum. Never use live tracing or generate symbols.
# https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/tracefmt-commands
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
. (Join-Path $PSScriptRoot 'collector-boundary.ps1')

function Assert-RemovalTraceDecodeToolApproval {
    param($Decode,[string]$Operation,$Request=$null)
    if($null -eq (Get-Command Assert-RemovalNativeOperation -ErrorAction SilentlyContinue)){
        throw 'Closed trace decode requires retained operation admission; no default tool launch.'
    }
    Assert-RemovalNativeOperation -Resource $Decode -Operation $Operation -Request $Request
}

function Read-RemovalDecodeFile {
    param([string]$Path,[long]$MaximumBytes,[switch]$Text,[switch]$AllowEmpty)
    Assert-FiniteToolPath $Path $false
    $file=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $sha=[Security.Cryptography.SHA256]::Create()
    try {
        $length=$file.Length
        if(($length -eq 0 -and -not$AllowEmpty) -or $length -gt $MaximumBytes){throw 'Decode file empty or exceeds explicit byte cap.'}
        $hash=[BitConverter]::ToString($sha.ComputeHash($file)).Replace('-','')
        $value=$null
        if($Text){
            $file.Position=0;$bytes=New-Object byte[] ([int]$length);$offset=0
            while($offset -lt $bytes.Length){
                $read=$file.Read($bytes,$offset,$bytes.Length-$offset)
                if($read -eq 0){throw 'Decode file became short during read.'};$offset+=$read
            }
            if($file.ReadByte() -ne -1 -or $file.Length -ne $length){throw 'Decode file changed during read.'}
            $value=[Text.UTF8Encoding]::new($false,$true).GetString($bytes).TrimStart([char]0xfeff)
        }
        [pscustomobject]@{Path=$Path;Bytes=$length;Sha256=$hash;Text=$value}
    } finally {$sha.Dispose();$file.Dispose()}
}

function Copy-RemovalDecodePin($Pin,[long]$MaximumBytes,[switch]$AllowEmpty) {
    if($null -eq $Pin -or $Pin.Path -isnot [string] -or
        ($Pin.Bytes -isnot [int] -and $Pin.Bytes -isnot [long]) -or $Pin.Bytes -lt 0 -or
        ($Pin.Bytes -eq 0 -and -not$AllowEmpty) -or $Pin.Bytes -gt $MaximumBytes -or
        $Pin.Sha256 -isnot [string] -or $Pin.Sha256 -cnotmatch '\A[A-Fa-f0-9]{64}\z'){
        throw 'Exact typed file path/length/SHA256 pin required.'
    }
    $actual=Read-RemovalDecodeFile $Pin.Path $MaximumBytes -AllowEmpty:$AllowEmpty
    if($actual.Bytes -ne $Pin.Bytes -or $actual.Sha256 -ine $Pin.Sha256){throw ('Decode input pin mismatch: '+$Pin.Path)}
    [pscustomobject]@{Path=$actual.Path;Bytes=$actual.Bytes;Sha256=$actual.Sha256}
}

function Assert-RemovalDecodeInputs($Decode) {
    foreach($entry in @(@{Name='Etl';Cap=16777216},@{Name='DriverPackage';Cap=1048576},
        @{Name='Tracefmt';Cap=268435456},@{Name='Python';Cap=268435456},@{Name='Reader';Cap=1048576},
        @{Name='ProbeReport';Cap=16777216})){
        $null=Copy-RemovalDecodePin $Decode.Pins.($entry.Name) $entry.Cap
    }
    $null=Copy-RemovalDecodePin $Decode.Pins.Probe.File 268435456
    $null=Copy-RemovalDecodePin $Decode.Pins.Probe.BuildManifest 1048576
    Assert-FiniteToolPath $Decode.Pins.Symbols.Directory $false
    $listed=@(Get-ChildItem -LiteralPath $Decode.Pins.Symbols.Directory -Force -ErrorAction Stop)
    if($listed.Count -ne $Decode.Pins.Symbols.Files.Count){throw 'TMF directory membership differs from exact reviewed pins.'}
    foreach($file in $listed){
        if($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            $file.Extension -ine '.tmf'){throw 'Only exact flat pinned TMF files are allowed.'}
        $match=@($Decode.Pins.Symbols.Files|Where-Object{$_.Path -ceq $file.FullName})
        if($match.Count -ne 1){throw 'Unpinned/duplicate TMF directory entry.'}
        $null=Copy-RemovalDecodePin $match[0] 1048576
    }
}

function Assert-RemovalDecodeStoppedSession($Session) {
    $native=$Session.NativeOwner;$readback=$Session.Readback;$receipt=$Session.StoppedReceipt
    if($null -eq $native -or $native.GetType().FullName -cne 'Moiren.RemovalTraceNativeV1.Owner' -or
        -not[Object]::ReferenceEquals($readback.Trace,$native) -or
        -not[Object]::ReferenceEquals($receipt,$native.StopReceipt) -or
        -not[Object]::ReferenceEquals($receipt.Owner,$native) -or $null -eq $readback.Initial -or
        $native.StartStatus -isnot [UInt32] -or $native.StartStatus -ne 0 -or
        $native.EnableStatus -isnot [UInt32] -or $native.EnableStatus -ne 0 -or
        -not $native.TraceOwned -or -not $native.TraceStopped -or -not $native.StopAttempted -or
        $receipt.Operation -cne 'Stop' -or $receipt.Status -isnot [UInt32] -or $receipt.Status -notin @(0,234) -or
        -not $receipt.StoppedConfirmed -or $receipt.OwnedHandle -ne $native.Handle -or
        $receipt.SessionGuid -ne $native.SessionGuid -or $receipt.SessionName -cne $native.Session -or
        $receipt.EtlPath -cne $native.EtlPath){
        throw 'Exact retained successful Start/Enable and confirmed closed STOP receipt required; no pending/foreign trace.'
    }
    # The initial DTO remains caller-visible; closed decoding must not accept a
    # replaced mode or silently strip flags from the original raw STOP payload.
    if($readback.Initial.LogFileMode -isnot [uint32] -or
        $readback.Initial.LogFileMode -ne [Moiren.RemovalTraceNativeV1.Api]::RequestedLogFileMode -or
        ($null -ne $receipt.Snapshot -and
            $receipt.Snapshot.LogFileMode -ne [Moiren.RemovalTraceNativeV1.Api]::RequestedLogFileMode)){
        throw 'Closed decode requires the exact explicitly requested native trace mode.'
    }
    return $receipt
}

function Assert-RemovalDecodeManagedReport($Decode) {
    $managed=$Decode.ManagedSession
    if($null -eq $managed -or -not[Object]::ReferenceEquals($managed.TraceSession,$Decode.TraceSession) -or
        -not[Object]::ReferenceEquals($managed.TraceReadback,$Decode.TraceSession.Readback) -or
        $null -eq $managed.ProbeOwner -or $null -eq $managed.LastResult){
        throw 'Original retained managed session/trace/probe/result required; no arbitrary report pairing.'
    }
    $self=[Diagnostics.Process]::GetCurrentProcess()
    if($managed.ControllerPid -ne $self.Id -or $managed.ControllerCreationUtcTicks -ne $self.StartTime.ToUniversalTime().Ticks){
        throw 'Decode belongs to another controller lifetime.'
    }
    $probe=$managed.ProbeOwner;$spec=$probe.Spec;$config=$managed.Config
    if(-not $probe.BindingKnown -or $probe.BindingError -or -not $probe.AdmissionWritten -or
        $spec.RunDirectory -cne $managed.RunDirectory -or $spec.Executable -cne $config.Executable -or
        $spec.ExpectedSha256 -ine $config.ExpectedSha256 -or
        $Decode.Pins.Probe.File.Path -cne $config.Executable -or
        $Decode.Pins.Probe.File.Sha256 -ine $config.ExpectedSha256 -or
        $probe.Ready.probe_pid -ne $probe.ProcessId -or $probe.Ready.probe_creation_utc_ticks -ne $probe.CreationUtcTicks){
        throw 'Managed probe executable/hash/identity/admission differs from original retained owner.'
    }
    Assert-RemovalProbeReady $spec $probe.Ready
    $state=Get-RemovalTaskObservation $managed.TaskOwner
    $terminal=Complete-RemovalProbeTerminal $managed.TaskOwner $state $probe $managed.LastResult
    if($terminal.Terminal -isnot [bool] -or -not $terminal.Terminal -or
        $terminal.ProbeOwnershipVerified -isnot [bool] -or -not $terminal.ProbeOwnershipVerified -or
        $terminal.NeedsObservation -isnot [bool] -or $terminal.NeedsObservation){
        throw 'Actual retained task/worker/probe is not terminal; detached terminal JSON is insufficient.'
    }
    $arguments=@($config.ProbeArguments)
    if($arguments.Count -lt 3 -or $arguments[0] -cne 'live-vad'){throw 'Original live-vad arguments required.'}
    $artifactIndices=@();$reportIndices=@()
    for($i=0;$i -lt $arguments.Count;$i++){
        if($arguments[$i] -ceq '--artifact-dir'){$artifactIndices+=,$i}
        if($arguments[$i] -ceq '--report'){$reportIndices+=,$i}
    }
    if($artifactIndices.Count -ne 1 -or $artifactIndices[0]+1 -ge $arguments.Count -or $reportIndices.Count -gt 1){
        throw 'Unique original artifact-dir/report selection required.'
    }
    $artifact=$arguments[$artifactIndices[0]+1];Assert-FiniteToolPath $artifact $false
    if(-not $artifact.StartsWith(($managed.RunDirectory+'\'),[StringComparison]::Ordinal)){
        throw 'Probe artifact directory must belong to the original managed run.'
    }
    $report=Join-Path $artifact 'report.json'
    if($Decode.Pins.ProbeReport.Path -cne $report -or ($reportIndices.Count -eq 1 -and
        ($reportIndices[0]+1 -ge $arguments.Count -or $arguments[$reportIndices[0]+1] -cne $report))){
        throw 'Pinned report is not the original live-vad artifact report.json.'
    }
    # Verify the original independently bound full argv still contains exactly
    # these live arguments. No report field itself proves process provenance.
    $separator=[Array]::IndexOf([string[]]$spec.Arguments,'--')
    if($separator -lt 0 -or $spec.Arguments.Count-$separator-1 -ne $arguments.Count){throw 'Managed argv/source selection differs.'}
    for($i=0;$i -lt $arguments.Count;$i++){
        if($spec.Arguments[$separator+1+$i] -cne $arguments[$i]){throw 'Managed argv/source selection differs.'}
    }
    $build=(Read-RemovalDecodeFile $Decode.Pins.Probe.BuildManifest.Path 1048576 -Text).Text|ConvertFrom-Json
    $properties=@($build.PSObject.Properties.Name)
    if($properties -contains 'SourceHead' -or $properties -contains 'Files'){
        # Current managed-probe-release-pin.json: match the unique EXE row,
        # never the PDB or a legacy fallback on a malformed current manifest.
        if($properties -notcontains 'SourceHead' -or $properties -notcontains 'Files' -or
            $properties -notcontains 'BuildExit' -or $properties -notcontains 'RustSourcesMatchSourceHead' -or
            $build.SourceHead -isnot [string] -or $build.Files -isnot [array] -or
            $build.Files.Count -lt 1 -or $build.Files.Count -gt 64 -or
            ($build.BuildExit -isnot [int] -and $build.BuildExit -isnot [long]) -or $build.BuildExit -ne 0 -or
            $build.RustSourcesMatchSourceHead -isnot [bool] -or -not$build.RustSourcesMatchSourceHead){
            throw 'Current managed probe release manifest schema/build result is invalid.'
        }
        $matches=@($build.Files|Where-Object{$_.Name -ieq 'e2e_latency_probe.exe'})
        if($matches.Count -ne 1 -or $matches[0].Name -cne 'e2e_latency_probe.exe'){
            throw 'Current release manifest requires exactly one e2e_latency_probe.exe row.'
        }
        $buildSource=$build.SourceHead;$buildFile=$matches[0]
    } else {
        # Explicit historical single-file alternate; never inferred from Files.
        foreach($name in @('Source','Path','Bytes','Sha256')){
            if($properties -notcontains $name){throw 'Historical single-file probe manifest schema is invalid.'}
        }
        $buildSource=$build.Source;$buildFile=$build
    }
    if($buildSource -isnot [string] -or $buildSource -cnotmatch '\A[0-9a-f]{40}\z' -or
        $buildSource -cne $Decode.Pins.Probe.SourceRevision -or
        $buildFile.Path -isnot [string] -or -not[IO.Path]::IsPathRooted($buildFile.Path) -or
        [IO.Path]::GetFileName($buildFile.Path) -cne 'e2e_latency_probe.exe' -or
        $buildFile.Sha256 -isnot [string] -or $buildFile.Sha256 -cnotmatch '\A[A-Fa-f0-9]{64}\z' -or
        $buildFile.Sha256 -ine $Decode.Pins.Probe.File.Sha256 -or
        ($buildFile.Bytes -isnot [int] -and $buildFile.Bytes -isnot [long]) -or
        $buildFile.Bytes -ne $Decode.Pins.Probe.File.Bytes){throw 'Original hash-pinned probe build manifest/source identity differs.'}
    return [pscustomobject]@{RunDirectory=$managed.RunDirectory;Nonce=$spec.Nonce;ControllerPid=$managed.ControllerPid
        ControllerCreationUtcTicks=$managed.ControllerCreationUtcTicks;WorkerPid=$spec.WorkerPid
        WorkerCreationUtcTicks=$spec.WorkerCreationUtcTicks;ProbePid=$probe.ProcessId;ProbeCreationUtcTicks=$probe.CreationUtcTicks
        ProbeExecutable=$spec.Executable;ProbeSha256=$spec.ExpectedSha256;ProbeArguments=$spec.Arguments
        ProbeExitCode=$terminal.ProbeExitCode;ReportPath=$report;SourceRevision=$buildSource}
}

function New-RemovalTraceDecode {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$TraceSession,[Parameter(Mandatory=$true)]$Pins,$ManagedSession=$null,
        [Parameter(Mandatory=$true)][string]$OutputDirectory,[hashtable]$ToolBoundary)
    $ErrorActionPreference='Stop'
    $finite=Join-Path $PSScriptRoot 'finite-tool.ps1'
    if((Get-FileHash -LiteralPath $finite -Algorithm SHA256).Hash -cne
        'C3621DC9C881519006E01619D2C65E053C3A1F0FBFC100C0E247E3064D16651B'){throw 'Frozen finite-tool hash differs.'}
    $receipt=Assert-RemovalDecodeStoppedSession $TraceSession
    Assert-FiniteToolPath $OutputDirectory $false
    if(Test-Path -LiteralPath $OutputDirectory){throw 'Create-new decode directory required.'}
    if($null -ne $ToolBoundary){
        if($ToolBoundary.Start -isnot [scriptblock] -or $ToolBoundary.Observe -isnot [scriptblock]){throw 'Both complete subprocess delegates required; no native fallback.'}
        $ToolBoundary=@{Start=$ToolBoundary.Start;Observe=$ToolBoundary.Observe}
    }
    if($Pins.SchemaVersion -ne 1){throw 'Decode pins schema 1 required.'}
    $copy=@{SchemaVersion=1}
    foreach($entry in @(@{Name='Etl';Cap=16777216},@{Name='DriverPackage';Cap=1048576},
        @{Name='Tracefmt';Cap=268435456},@{Name='Python';Cap=268435456},@{Name='Reader';Cap=1048576},
        @{Name='ProbeReport';Cap=16777216})){
        $copy[$entry.Name]=Copy-RemovalDecodePin $Pins.($entry.Name) $entry.Cap
    }
    if($copy.Etl.Path -cne $receipt.EtlPath -or [IO.Path]::GetExtension($copy.Etl.Path) -ine '.etl' -or
        [IO.Path]::GetFileName($copy.Tracefmt.Path) -ine 'tracefmt.exe' -or
        [IO.Path]::GetExtension($copy.Python.Path) -ine '.exe' -or
        [IO.Path]::GetFileName($copy.Reader.Path) -cne 'vad-removal-stages.py'){
        throw 'Exact closed ETL/tool/reader identity differs.'
    }
    $package=(Read-RemovalDecodeFile $copy.DriverPackage.Path 1048576 -Text).Text|ConvertFrom-Json
    if($package.SchemaVersion -ne 1 -or $package.DriverVersion -cne '3.1.1.613' -or
        $Pins.Symbols.DriverRevision -cnotmatch '\A[0-9a-f]{40}\z' -or $Pins.Symbols.DriverRevision -cne $package.Revision -or
        $Pins.Symbols.PdbSha256 -cnotmatch '\A[A-Fa-f0-9]{64}\z' -or $Pins.Symbols.PdbSha256 -ine $package.PdbSha256){
        throw 'Reviewed 613 package revision/PDB provenance differs from symbol pins.'
    }
    if($Pins.Probe.SourceRevision -cnotmatch '\A[0-9a-f]{40}\z'){throw 'Original probe source revision required; never relabel source.'}
    $copy.Probe=[pscustomobject]@{File=(Copy-RemovalDecodePin $Pins.Probe.File 268435456)
        BuildManifest=(Copy-RemovalDecodePin $Pins.Probe.BuildManifest 1048576);SourceRevision=$Pins.Probe.SourceRevision}
    $files=@();$total=[long]0
    if($Pins.Symbols.Files.Count -lt 1 -or $Pins.Symbols.Files.Count -gt 64){throw 'Explicit bounded complete TMF membership required.'}
    foreach($file in $Pins.Symbols.Files){$p=Copy-RemovalDecodePin $file 1048576;$files+=,$p;$total+=$p.Bytes}
    if($total -gt 16777216){throw 'TMF input aggregate cap exceeded.'}
    $copy.Symbols=[pscustomobject]@{Directory=$Pins.Symbols.Directory;Files=$files
        DriverRevision=$Pins.Symbols.DriverRevision;PdbSha256=$Pins.Symbols.PdbSha256.ToUpperInvariant()}
    $decode=[pscustomobject]@{TraceSession=$TraceSession;ManagedSession=$ManagedSession;StopReceipt=$receipt;Pins=$copy;ToolBoundary=$ToolBoundary
        OutputDirectory=$OutputDirectory;TracePath=(Join-Path $OutputDirectory 'driver.txt')
        SummaryPath=(Join-Path $OutputDirectory 'driver.txt.sum');AnalysisPath=(Join-Path $OutputDirectory 'association.json')
        ReceiptPath=(Join-Path $OutputDirectory 'decode-receipt.json');ReceiptPin=$null
        Attempted=$false;Completed=$false;Failed=$false;AssociationAvailable=$false;Analysis=$null
        RawTraceLossKnown=$false;RawTraceLoss='unknown';NativePassed=$false;Qualified=$false;ManagedProvenance=$null
        ToolOwners=(New-Object Collections.Generic.List[object]);Steps=(New-Object Collections.Generic.List[object])
        OutputPins=(New-Object Collections.Generic.List[object]);Errors=(New-Object Collections.Generic.List[string])}
    Assert-RemovalDecodeInputs $decode
    $decode.ManagedProvenance=Assert-RemovalDecodeManagedReport $decode
    return $decode
}

function Add-RemovalDecodeError($Decode,[string]$Message) {
    $Decode.Failed=$true
    if($Decode.Errors.Count -lt 16 -and -not$Decode.Errors.Contains($Message)){$Decode.Errors.Add($Message)}
}

function Invoke-RemovalDecodeTool($Decode,$Request,[switch]$AllowReaderRefusal) {
    $step=[pscustomobject]@{Request=$Request;Owner=$null;Receipt=$null;DispatchAttempted=$false}
    $Decode.Steps.Add($step)
    try {
        if($null -ne $Decode.ToolBoundary){$step.DispatchAttempted=$true;$owner=&$Decode.ToolBoundary.Start $Request}
        else {
            Assert-RemovalTraceDecodeToolApproval $Decode 'DecodeTool' $Request
            $step.DispatchAttempted=$true
            $owner=Start-FiniteTool -Executable $Request.Executable -ExpectedSha256 $Request.ExpectedSha256 `
                -OutputDirectory $Request.OutputDirectory -ArgumentList $Request.ArgumentList -TimeoutSeconds 15 -MaxLogBytes 1048576
        }
    } catch {
        $retained=$_.Exception.Data['ToolOwner']
        if($null -ne $retained){$Decode.ToolOwners.Add($retained);$step.Owner=$retained}
        throw
    }
    if($null -eq $owner -or $owner -is [Array]){throw 'Subprocess dispatch returned no single exact owner; never retry.'}
    $Decode.ToolOwners.Add($owner);$step.Owner=$owner
    $receipt=if($null -ne $Decode.ToolBoundary){&$Decode.ToolBoundary.Observe $owner $Decode}else{Observe-FiniteTool $owner}
    $step.Receipt=$receipt
    $allowed=$receipt.ExitCode -is [int] -and ($receipt.ExitCode -eq 0 -or ($AllowReaderRefusal -and $receipt.ExitCode -eq 1))
    if($owner.Process -isnot [Diagnostics.Process] -or -not $owner.IdentityConfirmed -or
        $owner.ProcessId -isnot [int] -or $owner.CreationUtcTicks -isnot [long] -or
        -not[Object]::ReferenceEquals($receipt.Owner,$owner) -or $receipt.ProcessId -ne $owner.ProcessId -or
        $receipt.CreationUtcTicks -ne $owner.CreationUtcTicks -or -not $allowed -or
        $receipt.ExitedConfirmed -isnot [bool] -or -not $receipt.ExitedConfirmed -or
        $receipt.TimedOut -isnot [bool] -or $receipt.TimedOut -or
        $receipt.OutputStable -isnot [bool] -or -not $receipt.OutputStable -or
        $receipt.NeedsObservation -isnot [bool] -or $receipt.NeedsObservation -or
        $receipt.LogLimitExceeded -isnot [bool] -or $receipt.LogLimitExceeded -or
        $receipt.Errors -isnot [Array] -or $receipt.Errors.Count -ne 0 -or
        $receipt.Success -isnot [bool] -or $receipt.Success -ne ($receipt.ExitCode -eq 0)){
        throw 'Original finite-tool receipt failed/late/unknown/capped; no downstream decode authority.'
    }
    # Original finite owner must agree, not just an equal-looking detached result.
    $actual=Get-FiniteToolReceipt $owner
    if($actual.ExitCode -ne $receipt.ExitCode -or -not $actual.ExitedConfirmed -or -not $actual.OutputStable -or
        $actual.TimedOut -or $actual.LogLimitExceeded -or $actual.Errors.Count -ne 0){throw 'Finite owner disagrees with returned observation.'}
    # Only a fully resolved original child/pipes/flush may be explicitly closed.
    # This releases our write handles before read-only hashing, not an unknown
    # child, descendant-absence claim, kill, or dispatch retry.
    Close-FiniteToolOwner $owner
    foreach($path in @($owner.StdoutPath,$owner.StderrPath)){
        $file=Read-RemovalDecodeFile $path 1048576 -AllowEmpty
        $Decode.OutputPins.Add([pscustomobject]@{Path=$file.Path;Bytes=$file.Bytes;Sha256=$file.Sha256})
    }
    return $receipt
}

function New-RemovalDecodeRequest($Decode,[string]$Step,[string]$Executable,[string]$Sha256,[string[]]$Arguments) {
    [pscustomobject]@{Step=$Step;Executable=$Executable;ExpectedSha256=$Sha256;ArgumentList=$Arguments
        OutputDirectory=(Join-Path $Decode.OutputDirectory ($Step+'-tool'));TimeoutSeconds=15;MaxLogBytes=1048576
        TracePath=$Decode.TracePath;SummaryPath=$Decode.SummaryPath;AnalysisPath=$Decode.AnalysisPath}
}

function Save-RemovalDecodeReceipt($Decode) {
    $steps=foreach($step in $Decode.Steps){
        $tool=$null;$actualOwner=$null
        if($null -ne $step.Owner){
            $o=$step.Owner
            $actualOwner=@{Executable=$o.Executable;ExecutableSha256=$o.ExecutableSha256;Arguments=$o.Arguments
                ProcessId=$o.ProcessId;CreationUtcTicks=$o.CreationUtcTicks;IdentityConfirmed=$o.IdentityConfirmed
                LaunchAttempted=$o.LaunchAttempted;LaunchUnknown=$o.LaunchUnknown;Closed=$o.Closed}
        }
        if($null -ne $step.Receipt){
            $r=$step.Receipt
            $tool=@{ProcessId=$r.ProcessId;CreationUtcTicks=$r.CreationUtcTicks;ExitCode=$r.ExitCode
                ExitedConfirmed=$r.ExitedConfirmed;TimedOut=$r.TimedOut;OutputStable=$r.OutputStable
                NeedsObservation=$r.NeedsObservation;Success=$r.Success;Errors=$r.Errors}
        }
        @{Step=$step.Request.Step;DispatchAttempted=$step.DispatchAttempted;Executable=$step.Request.Executable;ExpectedSha256=$step.Request.ExpectedSha256
            Arguments=$step.Request.ArgumentList;ActualOwner=$actualOwner;Tool=$tool}
    }
    $r=[ordered]@{SchemaVersion=1;Qualified=$false;NativePassed=$false;Completed=$Decode.Completed;Failed=$Decode.Failed
        ExecutionMode=$(if($null-ne$Decode.ToolBoundary){'InjectedSubprocessBoundary'}else{'DefaultFiniteTool'})
        AssociationAvailable=$Decode.AssociationAvailable;InputPins=$Decode.Pins;OutputPins=@($Decode.OutputPins.ToArray())
        ManagedProvenance=$Decode.ManagedProvenance
        Trace=@{Session=$Decode.StopReceipt.SessionName;SessionGuid=$Decode.StopReceipt.SessionGuid.ToString()
            OwnedHandle=$Decode.StopReceipt.OwnedHandle;StopStatus=$Decode.StopReceipt.Status
            StoppedConfirmed=$Decode.StopReceipt.StoppedConfirmed;FinalCountersKnown=$Decode.StopReceipt.FinalCountersKnown}
        RawTraceLossKnown=$Decode.RawTraceLossKnown;RawTraceLoss=$Decode.RawTraceLoss
        Steps=@($steps);Errors=@($Decode.Errors.ToArray())
        Limitations=@('Decoded subset is not native/audio/physical qualification.'
            'Raw loss is derived only from the retained STOP receipt plus readback, never decoder statistics.'
            'Probe source and TMF/PDB derivation are reviewed pinned provenance, not inferred from trace text.'
            'Finite stdout/stderr cap does not hard-bound tracefmt output-file growth; file caps are checked after exit.'
            'Synchronous file/hash reads are not hard process deadlines; unresolved tool owners remain retained.')}
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $r -Depth 16))
    if($bytes.Length -gt 1048576){throw 'Decode association receipt exceeds independent 1 MiB cap.'}
    $file=[IO.File]::Open($Decode.ReceiptPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try {$file.Write($bytes,0,$bytes.Length);$file.Flush($true)} finally {$file.Dispose()}
    $sha=[Security.Cryptography.SHA256]::Create()
    try {$expected=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')} finally {$sha.Dispose()}
    $actual=Read-RemovalDecodeFile $Decode.ReceiptPath 1048576
    if($actual.Bytes -ne $bytes.Length -or $actual.Sha256 -cne $expected){throw 'Original decode receipt readback differs.'}
    $Decode.ReceiptPin=[pscustomobject]@{Path=$actual.Path;Bytes=$actual.Bytes;Sha256=$actual.Sha256}
}

function Invoke-RemovalTraceDecode {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Decode)
    $ErrorActionPreference='Stop'
    if($Decode.Attempted){throw 'Decode already attempted; retain original owners/outputs, never retry.'}
    if($null -eq $Decode.ToolBoundary){Assert-RemovalTraceDecodeToolApproval $Decode 'DecodeBegin'}
    $Decode.Attempted=$true
    try {
        $receipt=Assert-RemovalDecodeStoppedSession $Decode.TraceSession
        if(-not[Object]::ReferenceEquals($receipt,$Decode.StopReceipt)){throw 'Retained STOP changed before decode.'}
        Assert-RemovalDecodeInputs $Decode
        $Decode.ManagedProvenance=Assert-RemovalDecodeManagedReport $Decode
        if(Test-Path -LiteralPath $Decode.OutputDirectory){throw 'Preserve existing decode outputs.'}
        $null=[IO.Directory]::CreateDirectory($Decode.OutputDirectory)
        # MORE_DATA confirms stop, not final counters. It may still yield a
        # useful diagnostic subset, but cannot acquire any raw-loss authority.
        $readback=$Decode.TraceSession.Readback
        if($receipt.Status -eq 0 -and $receipt.FinalCountersKnown -and $readback.FinalCountersKnown -and -not $readback.Invalid){
            $Decode.RawTraceLoss=[pscustomobject]@{EventsLost=$receipt.Snapshot.EventsLost
                LogBuffersLost=$receipt.Snapshot.LogBuffersLost;RealTimeBuffersLost=$receipt.Snapshot.RealTimeBuffersLost}
            $Decode.RawTraceLossKnown=$true
        }
        $request=New-RemovalDecodeRequest $Decode 'tracefmt' $Decode.Pins.Tracefmt.Path $Decode.Pins.Tracefmt.Sha256 `
            @($Decode.Pins.Etl.Path,'-p',$Decode.Pins.Symbols.Directory,'-o',$Decode.TracePath)
        $null=Invoke-RemovalDecodeTool $Decode $request
        Assert-RemovalDecodeInputs $Decode
        foreach($path in @($Decode.TracePath,$Decode.SummaryPath)){
            $f=Read-RemovalDecodeFile $path 16777216 -Text
            $Decode.OutputPins.Add([pscustomobject]@{Path=$f.Path;Bytes=$f.Bytes;Sha256=$f.Sha256})
        }
        $request=New-RemovalDecodeRequest $Decode 'associate' $Decode.Pins.Python.Path $Decode.Pins.Python.Sha256 `
            @($Decode.Pins.Reader.Path,'--trace',$Decode.TracePath,'--summary',$Decode.SummaryPath,
                '--report',$Decode.Pins.ProbeReport.Path,'--output',$Decode.AnalysisPath,'--max-input-bytes','16777216')
        $result=Invoke-RemovalDecodeTool $Decode $request -AllowReaderRefusal
        Assert-RemovalDecodeInputs $Decode
        foreach($pin in $Decode.OutputPins){$null=Copy-RemovalDecodePin $pin 16777216 -AllowEmpty}
        $analysis=Read-RemovalDecodeFile $Decode.AnalysisPath 1048576 -Text
        $Decode.OutputPins.Add([pscustomobject]@{Path=$analysis.Path;Bytes=$analysis.Bytes;Sha256=$analysis.Sha256})
        $Decode.Analysis=$analysis.Text|ConvertFrom-Json
        if($Decode.Analysis.qualified -isnot [bool] -or $Decode.Analysis.qualified -or
            $Decode.Analysis.trace_association_available -isnot [bool]){throw 'Offline reader returned invalid diagnostic semantics.'}
        if($result.ExitCode -ne 0 -or -not $Decode.Analysis.trace_association_available){throw 'Offline reader refused decoded association; original reasons retained.'}
        $Decode.AssociationAvailable=$true;$Decode.Completed=$true
    } catch {Add-RemovalDecodeError $Decode $_.Exception.Message}
    finally {
        if(Test-Path -LiteralPath $Decode.OutputDirectory -PathType Container){
            try {Save-RemovalDecodeReceipt $Decode} catch {Add-RemovalDecodeError $Decode ('Receipt persistence failed: '+$_.Exception.Message);$Decode.Completed=$false}
        }
    }
    return $Decode
}
