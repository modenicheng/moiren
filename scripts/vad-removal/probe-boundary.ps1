#requires -Version 5.1
# Definitions only. Native dispatch remains owned by the reviewed run controller.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'task-boundary.ps1')
. (Join-Path $PSScriptRoot 'finite-tool.ps1')

function Write-RemovalJsonOnce([string]$Path,$Value) {
    Assert-RemovalTaskPath $Path
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $Value -Depth 12 -Compress))
    if($bytes.Length -gt 8192){throw 'Removal receipt exceeds 8192-byte cap.'}
    $temporary=$Path+'.partial'
    Assert-RemovalTaskPath $temporary
    $file=[IO.File]::Open($temporary,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try {$file.Write($bytes,0,$bytes.Length);$file.Flush()} finally {$file.Dispose()}
    # Move has no overwrite on .NET Framework. A failed publication preserves
    # the partial file and cannot silently replace an earlier admission.
    [IO.File]::Move($temporary,$Path)
}

function Read-RemovalJsonBounded([string]$Path) {
    Assert-RemovalTaskPath $Path
    $file=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try {
        if($file.Length -lt 2 -or $file.Length -gt 8192){throw 'Removal receipt length invalid.'}
        $bytes=New-Object byte[] ([int]$file.Length);$offset=0
        while($offset -lt $bytes.Length){
            $count=$file.Read($bytes,$offset,$bytes.Length-$offset)
            if($count -eq 0){throw 'Incomplete removal receipt.'};$offset+=$count
        }
        if($file.ReadByte() -ne -1){throw 'Removal receipt grew during read.'}
        [Text.UTF8Encoding]::new($false,$true).GetString($bytes)|ConvertFrom-Json
    } finally {$file.Dispose()}
}

function New-RemovalProbeSpec {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$Executable,
        [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedSha256,
        [Parameter(Mandatory=$true)][string]$RunDirectory,
        [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{32}$')][string]$Nonce,
        [Parameter(Mandatory=$true)][ValidateRange(1,65535)][int]$SessionId,
        [Parameter(Mandatory=$true)][ValidatePattern('^S-1-5-21-[0-9-]+$')][string]$Sid,
        [Parameter(Mandatory=$true)][ValidateRange(1,2147483647)][int]$ControllerPid,
        [Parameter(Mandatory=$true)][ValidateRange(1,[long]::MaxValue)][long]$ControllerCreationUtcTicks,
        [Parameter(Mandatory=$true)][ValidateRange(1,2147483647)][int]$WorkerPid,
        [Parameter(Mandatory=$true)][ValidateRange(1,[long]::MaxValue)][long]$WorkerCreationUtcTicks,
        [Parameter(Mandatory=$true)][string[]]$ProbeArguments)
    Assert-FiniteToolPath $Executable $true;Assert-RemovalTaskPath $RunDirectory
    if(-not(Test-Path -LiteralPath $RunDirectory -PathType Container) -or $ProbeArguments.Count -eq 0 -or
        $ProbeArguments[0] -cne 'live-vad' -or
        (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash -ine $ExpectedSha256){throw 'Pinned managed live-vad probe required.'}
    $argv=@('managed-removal','--run-dir',$RunDirectory,'--nonce',$Nonce,
        '--controller-pid',[string]$ControllerPid,'--controller-creation-ticks',[string]$ControllerCreationUtcTicks,
        '--worker-pid',[string]$WorkerPid,'--worker-creation-ticks',[string]$WorkerCreationUtcTicks,
        '--admission-timeout-ms','15000','--')+$ProbeArguments
    $quoted=foreach($argument in $argv){
        if($null -eq $argument -or $argument -match '[\x00\r\n]'){throw 'Unsafe managed probe argument.'}
        Quote-FiniteToolArgument $argument
    }
    $line=$quoted -join ' '
    if($Executable.Length+$line.Length+3 -gt 32766){throw 'Managed probe command line too long.'}
    [pscustomobject]@{Executable=$Executable;ExpectedSha256=$ExpectedSha256.ToUpperInvariant();RunDirectory=$RunDirectory
        Nonce=$Nonce;SessionId=$SessionId;Sid=$Sid;ControllerPid=$ControllerPid;ControllerCreationUtcTicks=$ControllerCreationUtcTicks
        WorkerPid=$WorkerPid;WorkerCreationUtcTicks=$WorkerCreationUtcTicks;Arguments=$argv;CommandLine=$line
        ObservationSeconds=120;AdmissionTimeoutMilliseconds=15000}
}

function New-RemovalProbeOwner {
    param([Parameter(Mandatory=$true)]$Spec)
    [pscustomobject]@{Spec=$Spec;Process=$null;Handle=[IntPtr]::Zero;ProcessId=$null;CreationUtcTicks=$null
        Ready=$null;BindingKnown=$false;BindingError=$null;AdmissionAttempted=$false;AdmissionWritten=$false
        ExitKnown=$false;ExitCode=$null;TimedOut=$false;Clock=[Diagnostics.Stopwatch]::StartNew()
        DeadlineMilliseconds=([long]$Spec.ObservationSeconds*1000);Closed=$false}
}

function Assert-RemovalProbeReady($Spec,$Receipt) {
    foreach($field in @('schema_version','controller_pid','controller_creation_utc_ticks','worker_pid','worker_creation_utc_ticks','session_id','probe_pid','probe_creation_utc_ticks')){
        if($null -eq $Receipt -or ($Receipt.$field -isnot [int] -and $Receipt.$field -isnot [long])){throw 'Typed integer receipt field required.'}
    }
    if($null -eq $Receipt -or $Receipt.schema_version -ne 1 -or $Receipt.nonce -cne $Spec.Nonce -or
        $Receipt.controller_pid -ne $Spec.ControllerPid -or $Receipt.controller_creation_utc_ticks -ne $Spec.ControllerCreationUtcTicks -or
        $Receipt.worker_pid -ne $Spec.WorkerPid -or $Receipt.worker_creation_utc_ticks -ne $Spec.WorkerCreationUtcTicks -or
        $Receipt.session_id -ne $Spec.SessionId -or
        ($Receipt.probe_pid -isnot [int] -and $Receipt.probe_pid -isnot [long]) -or
        $Receipt.probe_pid -lt 1 -or $Receipt.probe_pid -gt [int]::MaxValue -or
        $Receipt.probe_creation_utc_ticks -isnot [long] -or $Receipt.probe_creation_utc_ticks -le 0){
        throw 'Managed probe receipt identity differs.'
    }
}

function Open-RemovalProbeProcess([int]$ProbePid) {[Diagnostics.Process]::GetProcessById($ProbePid)}

function Read-RemovalProbeMetadata([Diagnostics.Process]$Process) {
    $rows=@(Get-CimInstance -ClassName Win32_Process -Filter ('ProcessId='+$Process.Id) -OperationTimeoutSec 10)
    if($rows.Count -ne 1){throw 'Exact live probe metadata unavailable.'}
    $token=Read-RemovalWorkerToken $Process
    @{Pid=$rows[0].ProcessId;ParentPid=$rows[0].ParentProcessId;Sid=$token.Sid
        IsAdministrator=$token.IsAdministrator;IsElevated=$token.IsElevated
        Executable=$rows[0].ExecutablePath;CommandLine=$rows[0].CommandLine}
}

function Attach-RemovalProbeProcess {
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[Parameter(Mandatory=$true,Position=1)]$Receipt)
    $ErrorActionPreference='Stop'
    if($Owner.Closed -or $null -ne $Owner.Process -or $Owner.BindingError){throw 'Probe binding already attempted; retain same owner.'}
    Assert-RemovalProbeReady $Owner.Spec $Receipt
    $Owner.Ready=$Receipt
    try {
        $Owner.Process=Open-RemovalProbeProcess -ProbePid $Receipt.probe_pid
        $p=$Owner.Process;$null=$p.Handle;$p.Refresh();$created=$p.StartTime.ToUniversalTime();$s=$Owner.Spec
        if($p.HasExited -or $p.Id -ne $Receipt.probe_pid -or $created.Ticks -ne $Receipt.probe_creation_utc_ticks -or
            $created.Ticks -lt $s.WorkerCreationUtcTicks -or $created -gt [DateTime]::UtcNow -or
            $p.SessionId -ne $s.SessionId -or $p.MainModule.FileName -cne $s.Executable){throw 'Probe retained handle/creation/session/image differs.'}
        $m=Read-RemovalProbeMetadata $p
        if($m.Pid -ne $p.Id -or $m.ParentPid -ne $s.WorkerPid -or $m.Sid -cne $s.Sid -or
            $m.IsAdministrator -isnot [bool] -or $m.IsAdministrator -or $m.IsElevated -isnot [bool] -or $m.IsElevated -or
            $m.Executable -cne $s.Executable -or
            ($m.CommandLine -cne ('"'+$s.Executable+'" '+$s.CommandLine) -and $m.CommandLine -cne ($s.Executable+' '+$s.CommandLine)) -or
            (Get-FileHash -LiteralPath $s.Executable -Algorithm SHA256).Hash -ine $s.ExpectedSha256){throw 'Probe command line/parent/token/hash differs.'}
        $p.Refresh();if($p.HasExited){throw 'Probe exited before admission.'}
        $Owner.Handle=$p.Handle;$Owner.ProcessId=[int]$p.Id;$Owner.CreationUtcTicks=$created.Ticks;$Owner.BindingKnown=$true
    } catch {$Owner.BindingError='Probe binding failed/unknown: '+$_.Exception.Message;throw}
}

function Admit-RemovalProbe {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    if(-not $Owner.BindingKnown -or $Owner.BindingError -or $Owner.AdmissionAttempted -or $Owner.Closed){throw 'No fresh independent live probe binding.'}
    $Owner.Process.Refresh();if($Owner.Process.HasExited){throw 'Probe already exited.'}
    $Owner.AdmissionAttempted=$true
    Write-RemovalJsonOnce (Join-Path $Owner.Spec.RunDirectory 'probe-admit.json') $Owner.Ready
    $Owner.AdmissionWritten=$true
}

function Observe-RemovalProbe {
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[ValidateRange(0,15000)][int]$ObservationMilliseconds=0)
    if($Owner.Closed){throw 'Probe owner closed.'}
    if(-not $Owner.ExitKnown -and $Owner.Clock.ElapsedMilliseconds -ge $Owner.DeadlineMilliseconds){$Owner.TimedOut=$true}
    if($null -ne $Owner.Process -and -not $Owner.ExitKnown){
        if($Owner.Process.WaitForExit($ObservationMilliseconds)){
            $Owner.ExitCode=[int]$Owner.Process.ExitCode;$Owner.ExitKnown=$true
        }
        if($Owner.Clock.ElapsedMilliseconds -ge $Owner.DeadlineMilliseconds){$Owner.TimedOut=$true}
    }
    [pscustomobject]@{Owner=$Owner;ExitKnown=$Owner.ExitKnown;ExitCode=$Owner.ExitCode;TimedOut=$Owner.TimedOut
        NeedsObservation=(-not $Owner.ExitKnown);NativePassed=$false;Qualified=$false}
}

function Complete-RemovalProbeTerminal($TaskOwner,$TaskState,$ProbeOwner,$WorkerResult) {
    $TaskTerminal=Test-RemovalTaskTerminal $TaskOwner $TaskState $WorkerResult
    $r=[ordered]@{Terminal=$false;NeedsObservation=$true;HasRun=$false;Owners=@('probe-or-worker-unresolved')
        TaskExitCode=$TaskTerminal.TaskExitCode;ProbeExitKnown=$false;ProbeExitCode=$null
        ProbeOwnershipVerified=$false;TimedOut=$false;NativePassed=$false;Qualified=$false}
    if($TaskTerminal.WorkerTerminal -isnot [bool] -or -not $TaskTerminal.WorkerTerminal -or $null -eq $ProbeOwner -or -not $ProbeOwner.BindingKnown -or
        $ProbeOwner.BindingError -or $null -eq $WorkerResult){return $r}
    $taskSpec=$TaskOwner.Spec;$probeSpec=$ProbeOwner.Spec
    if($taskSpec.Nonce -cne $probeSpec.Nonce -or $taskSpec.RunDirectory -cne $probeSpec.RunDirectory -or
        $taskSpec.SessionId -ne $probeSpec.SessionId -or $taskSpec.Sid -cne $probeSpec.Sid -or
        $TaskOwner.WorkerPid -ne $probeSpec.WorkerPid -or
        $TaskOwner.WorkerCreationUtcTicks -ne $probeSpec.WorkerCreationUtcTicks){return $r}
    $probe=Observe-RemovalProbe $ProbeOwner
    $r.TimedOut=$probe.TimedOut
    if(-not $probe.ExitKnown -or $WorkerResult.ProbePid -ne $ProbeOwner.ProcessId -or
        $WorkerResult.ProbeCreationUtcTicks -ne $ProbeOwner.CreationUtcTicks -or
        $WorkerResult.ProbeExitKnown -isnot [bool] -or -not $WorkerResult.ProbeExitKnown -or
        ($WorkerResult.ProbeExitCode -isnot [int] -and $WorkerResult.ProbeExitCode -isnot [long]) -or
        $WorkerResult.ProbeExitCode -ne $probe.ExitCode -or $WorkerResult.OutputStable -isnot [bool] -or -not $WorkerResult.OutputStable -or
        $WorkerResult.Owners -isnot [Array] -or $WorkerResult.Owners.Count -ne 0){return $r}
    # Exited with an error is terminal ownership, not native diagnostic success.
    # Raw trace, audio report, safety readback and loss acceptance live above it.
    $r.Terminal=$true;$r.NeedsObservation=$false;$r.HasRun=$true;$r.Owners=@()
    $r.ProbeExitKnown=$true;$r.ProbeExitCode=$probe.ExitCode;$r.ProbeOwnershipVerified=$true
    return $r
}
