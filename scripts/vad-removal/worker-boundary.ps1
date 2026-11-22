#requires -Version 5.1
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'probe-boundary.ps1')

function New-RemovalWorkerProbeOwner {
    param([Parameter(Mandatory=$true)]$Spec,[ValidateRange(1,16777216)][long]$MaxLogBytes=1048576)
    Initialize-FiniteToolSink
    [pscustomobject]@{Spec=$Spec;Process=$null;Handle=[IntPtr]::Zero;ProcessId=$null;CreationUtcTicks=$null
        LaunchAttempted=$false;LaunchUnknown=$false;IdentityConfirmed=$false;ExitKnown=$false;ExitCode=$null
        Files=@();Copies=@();Flushes=@();Sinks=@();Budget=[Moiren.RemovalFiniteToolV1.Budget]::new($MaxLogBytes)
        StdoutPath=(Join-Path $Spec.RunDirectory 'probe-stdout.log');StderrPath=(Join-Path $Spec.RunDirectory 'probe-stderr.log')
        OutputStable=$false;OutputObserved=$false;TimedOut=$false;Clock=$null;DeadlineMilliseconds=([long]$Spec.ObservationSeconds*1000)
        Closed=$false;Errors=New-Object Collections.Generic.List[string]}
}

function Start-RemovalWorkerProbe {
    param([Parameter(Mandatory=$true)]$Owner,[scriptblock]$ProcessFactory)
    if($Owner.LaunchAttempted -or $Owner.Errors.Count -or $Owner.Closed){throw 'Probe launch already attempted/refused; retain owner.'}
    $s=$Owner.Spec
    Assert-FiniteToolPath $s.Executable $true;Assert-RemovalTaskPath $s.RunDirectory
    if((Get-FileHash -LiteralPath $s.Executable -Algorithm SHA256).Hash -ine $s.ExpectedSha256){throw 'Probe executable changed.'}
    $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=$s.Executable;$start.Arguments=$s.CommandLine
    $start.WorkingDirectory=$s.RunDirectory;$start.UseShellExecute=$false;$start.CreateNoWindow=$true
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    try {
        foreach($path in @($Owner.StdoutPath,$Owner.StderrPath)){
            Assert-RemovalTaskPath $path
            $Owner.Files+=,[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
        }
        $Owner.Process=if($null -ne $ProcessFactory){&$ProcessFactory}else{[Diagnostics.Process]::new()}
        $Owner.Process.StartInfo=$start;$Owner.Clock=[Diagnostics.Stopwatch]::StartNew()
        $Owner.LaunchAttempted=$true;$Owner.LaunchUnknown=$true
        if(-not $Owner.Process.Start()){throw 'Probe dispatch unconfirmed.'}
        $Owner.LaunchUnknown=$false
        Initialize-RemovalWorkerProbeObservation $Owner
    } catch {Add-FiniteToolError $Owner ('Probe launch/identity/output setup failed: '+$_.Exception.Message)}
}

function Initialize-RemovalWorkerProbeObservation($Owner) {
    # Only this retained Process may complete a failed/ambiguous initial bind.
    # Never GetProcessById, Start again, or forget a possibly dispatched child.
    if(-not $Owner.IdentityConfirmed){
        $Owner.Handle=$Owner.Process.Handle;$Owner.ProcessId=$Owner.Process.Id
        $Owner.CreationUtcTicks=$Owner.Process.StartTime.ToUniversalTime().Ticks
        if($Owner.Handle -eq [IntPtr]::Zero -or $Owner.ProcessId -le 0 -or $Owner.CreationUtcTicks -le 0){throw 'Probe process identity unavailable.'}
        $Owner.IdentityConfirmed=$true
    }
    while($Owner.Copies.Count -lt 2){
        $index=$Owner.Copies.Count
        $stream=if($index -eq 0){$Owner.Process.StandardOutput.BaseStream}else{$Owner.Process.StandardError.BaseStream}
        $sink=[Moiren.RemovalFiniteToolV1.Sink]::new($Owner.Files[$index],$Owner.Budget)
        $Owner.Sinks+=,$sink;$Owner.Copies+=,$stream.CopyToAsync($sink,4096)
    }
}

function Observe-RemovalWorkerProbe {
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[ValidateRange(0,15000)][int]$ObservationMilliseconds=0)
    if($Owner.Closed){throw 'Worker probe owner closed.'}
    if($Owner.LaunchAttempted -and $null -ne $Owner.Process -and (-not $Owner.IdentityConfirmed -or $Owner.Copies.Count -lt 2)){
        try {Initialize-RemovalWorkerProbeObservation $Owner}
        catch {Add-FiniteToolError $Owner 'Original probe observation identity/pipes unavailable; retain exact process.'}
    }
    if($Owner.IdentityConfirmed -and -not $Owner.OutputObserved){
        $end=$Owner.Clock.ElapsedMilliseconds+$ObservationMilliseconds
        try {
            if($Owner.Clock.ElapsedMilliseconds -ge $Owner.DeadlineMilliseconds){$Owner.TimedOut=$true}
            if(-not $Owner.ExitKnown -and $Owner.Process.WaitForExit($ObservationMilliseconds)){
                $Owner.ExitCode=[int]$Owner.Process.ExitCode;$Owner.ExitKnown=$true
            }
            if($Owner.ExitKnown){
                $drained=$Owner.Copies.Count -eq 2
                foreach($copy in $Owner.Copies){
                    $left=[int][Math]::Max(0,$end-$Owner.Clock.ElapsedMilliseconds)
                    if(-not $copy.IsCompleted -and -not $copy.Wait($left)){$drained=$false;continue}
                    $null=$copy.GetAwaiter().GetResult()
                }
                if($drained){
                    while($Owner.Flushes.Count -lt $Owner.Files.Count){$Owner.Flushes+=,$Owner.Files[$Owner.Flushes.Count].FlushAsync()}
                    $flushed=$Owner.Flushes.Count -eq 2
                    foreach($flush in $Owner.Flushes){
                        $left=[int][Math]::Max(0,$end-$Owner.Clock.ElapsedMilliseconds)
                        if(-not $flush.IsCompleted -and -not $flush.Wait($left)){$flushed=$false;continue}
                        $null=$flush.GetAwaiter().GetResult()
                    }
                    if($flushed){
                        $sinkFailed=@($Owner.Sinks|Where-Object{$_.Failed}).Count -ne 0
                        if($sinkFailed){Add-FiniteToolError $Owner 'Probe output persistence failed.'}
                        $Owner.OutputStable=(-not $sinkFailed -and -not $Owner.Budget.Exceeded)
                        $Owner.OutputObserved=$true
                    }
                }
            }
            if($Owner.Clock.ElapsedMilliseconds -ge $Owner.DeadlineMilliseconds){$Owner.TimedOut=$true}
        } catch {Add-FiniteToolError $Owner ('Probe observation/drain failed: '+$_.Exception.Message)}
    }
    [pscustomobject]@{Owner=$Owner;ExitKnown=$Owner.ExitKnown;ExitCode=$Owner.ExitCode;OutputStable=$Owner.OutputStable
        TimedOut=$Owner.TimedOut;LogLimitExceeded=$Owner.Budget.Exceeded;NeedsObservation=(-not $Owner.ExitKnown -or -not $Owner.OutputObserved -or
            $Owner.Copies.Count -ne 2 -or @($Owner.Copies|Where-Object{-not $_.IsCompleted}).Count -ne 0 -or
            $Owner.Flushes.Count -ne 2 -or @($Owner.Flushes|Where-Object{-not $_.IsCompleted}).Count -ne 0)
        NativePassed=$false;Qualified=$false;Errors=@($Owner.Errors.ToArray())}
}

function Close-RemovalWorkerProbeOwner {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    if($Owner.Closed){return}
    if($Owner.LaunchAttempted -and ((Observe-RemovalWorkerProbe $Owner).NeedsObservation -or -not $Owner.ExitKnown)){
        throw 'Unresolved exact probe/pipes retained; no kill/dispose/relaunch.'
    }
    foreach($file in $Owner.Files){$file.Dispose()}
    if($null -ne $Owner.Process){$Owner.Process.Dispose()}
    $Owner.Closed=$true
}

function Assert-RemovalWorkerContext($Spec,[string]$TaskPath,[string]$WorkerPath,[string]$WorkerSha256) {
    Assert-RemovalTaskHost
    $p=[Diagnostics.Process]::GetCurrentProcess();$token=Read-RemovalWorkerToken $p
    if($p.Id -ne $Spec.WorkerPid -or $p.StartTime.ToUniversalTime().Ticks -ne $Spec.WorkerCreationUtcTicks -or
        $p.SessionId -ne $Spec.SessionId -or $token.Sid -cne $Spec.Sid -or $token.IsAdministrator -or $token.IsElevated -or
        $TaskPath -cne ('\Moiren-Removal613-'+$Spec.Nonce) -or
        (Get-FileHash -LiteralPath $WorkerPath -Algorithm SHA256).Hash -ine $WorkerSha256){throw 'Wrong worker process/token/task/source context.'}
    @{Nonce=$Spec.Nonce;TaskPath=$TaskPath;Host=[Environment]::MachineName;Sid=$token.Sid;SessionId=$p.SessionId
        IsAdministrator=$false;Pid=$p.Id;CreationUtcTicks=$p.StartTime.ToUniversalTime().Ticks;WorkerSha256=$WorkerSha256}
}

function Invoke-RemovalWorker {
    param([Parameter(Mandatory=$true)]$Spec,[Parameter(Mandatory=$true)][string]$TaskPath,
        [Parameter(Mandatory=$true)][string]$WorkerPath,[Parameter(Mandatory=$true)][string]$WorkerSha256)
    $ErrorActionPreference='Stop'
    $identity=Assert-RemovalWorkerContext $Spec $TaskPath $WorkerPath $WorkerSha256
    Write-RemovalJsonOnce (Join-Path $Spec.RunDirectory 'worker-start.json') $identity
    $owner=New-RemovalWorkerProbeOwner -Spec $Spec
    # Keep this owner in the script scope even when receipt publication throws.
    # Neither an exception nor a deadline authorizes killing a native owner.
    $script:RemovalWorkerProbeOwner=$owner
    Start-RemovalWorkerProbe -Owner $owner
    $launch=@{Nonce=$Spec.Nonce;ProbeIdentityKnown=$owner.IdentityConfirmed;ProbePid=$owner.ProcessId
        ProbeCreationUtcTicks=$owner.CreationUtcTicks;LaunchAttempted=$owner.LaunchAttempted;LaunchUnknown=$owner.LaunchUnknown}
    try {Write-RemovalJsonOnce (Join-Path $Spec.RunDirectory 'worker-probe-start.json') $launch}
    catch {Add-FiniteToolError $owner 'Probe-start receipt publication failed; keep original child.'}
    $pendingWritten=$false
    while($true){
        try {$observed=Observe-RemovalWorkerProbe $owner -ObservationMilliseconds 100}
        catch {
            Add-FiniteToolError $owner 'Probe observation failed; retain exact child and retry observation only.'
            Start-Sleep -Milliseconds 100
            continue
        }
        if($observed.NeedsObservation -and $observed.TimedOut -and -not $pendingWritten){
            $pendingWritten=$true
            try {Write-RemovalJsonOnce (Join-Path $Spec.RunDirectory 'worker-pending.json') $launch}
            catch {Add-FiniteToolError $owner 'Pending receipt publication failed; keep original child.'}
        }
        if($observed.NeedsObservation){Start-Sleep -Milliseconds 100}
        # The observation deadline is a failed diagnostic, not authorization to
        # abandon/terminate a kernel-owning probe. Keep this one worker waiting.
        # The controller can return a pending receipt and retain its own handle.
        if(-not $observed.NeedsObservation -or (-not $owner.LaunchAttempted -and -not $owner.IdentityConfirmed)){break}
    }
    $result=@{}
    foreach($key in $identity.Keys){$result[$key]=$identity[$key]}
    $result.ProbePid=$owner.ProcessId;$result.ProbeCreationUtcTicks=$owner.CreationUtcTicks
    $result.ProbeExitKnown=$observed.ExitKnown;$result.ProbeExitCode=$observed.ExitCode
    $result.OutputStable=$observed.OutputStable;$result.TimedOut=$observed.TimedOut
    $result.LogLimitExceeded=$observed.LogLimitExceeded;$result.Errors=$observed.Errors
    $result.Owners=@();if($observed.NeedsObservation){$result.Owners=@('exact-probe-or-pipes-retained')}
    $result.WorkerExitCode=if($observed.ExitKnown -and $observed.ExitCode -eq 0 -and $observed.OutputStable -and
        -not $observed.TimedOut -and -not $observed.LogLimitExceeded -and $observed.Errors.Count -eq 0){0}else{1}
    $result.NativePassed=$false;$result.Qualified=$false
    Write-RemovalJsonOnce (Join-Path $Spec.RunDirectory 'worker-result.json') $result
    if(-not $observed.NeedsObservation){Close-RemovalWorkerProbeOwner $owner}
    return $result
}
