#requires -Version 5.1
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'probe-boundary.ps1')

function New-RemovalManagedSession {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$RunDirectory,[string]$SourceDirectory=$PSScriptRoot,
        [Parameter(Mandatory=$true)][string]$Executable,
        [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedSha256,
        [Parameter(Mandatory=$true)][string[]]$ProbeArguments,
        [Parameter(Mandatory=$true)][ValidateRange(1,65535)][int]$SessionId,
        [Parameter(Mandatory=$true)][ValidatePattern('^S-1-5-21-[0-9-]+$')][string]$Sid,
        [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{32}$')][string]$Nonce)
    Assert-RemovalTaskPath $RunDirectory;Assert-RemovalTaskPath $SourceDirectory;Assert-FiniteToolPath $Executable $true
    if($Sid -cne 'S-1-5-21-3080296863-1242338503-1346362208-1001'){throw 'Wrong limited target principal.'}
    if(Test-Path -LiteralPath $RunDirectory){throw 'Create-new managed run directory required.'}
    if($ProbeArguments.Count -eq 0 -or $ProbeArguments[0] -cne 'live-vad' -or
        (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash -ine $ExpectedSha256){throw 'Pinned live-vad probe required.'}
    $controller=[Diagnostics.Process]::GetCurrentProcess()
    $libraries=@();$names=@('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1')
    $sources=@{}
    foreach($name in ($names+@('worker.ps1'))){
        $source=Join-Path $SourceDirectory $name;Assert-RemovalTaskPath $source
        $sources[$name]=[IO.File]::ReadAllBytes($source)
    }
    # Only five small pinned scripts are staged. The probe/OBS executable stays
    # at its fixed installation path; a run never creates another EXE tree.
    $null=[IO.Directory]::CreateDirectory($RunDirectory)
    foreach($name in ($names+@('worker.ps1'))){
        $destination=Join-Path $RunDirectory $name
        $file=[IO.File]::Open($destination,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try {$file.Write($sources[$name],0,$sources[$name].Length)} finally {$file.Dispose()}
        if($name -cin $names){$libraries+=@{Name=$name;Sha256=(Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash}}
    }
    $config=@{SchemaVersion=1;Nonce=$Nonce;RunDirectory=$RunDirectory;SessionId=$SessionId;Sid=$Sid
        ControllerPid=$controller.Id;ControllerCreationUtcTicks=$controller.StartTime.ToUniversalTime().Ticks
        Executable=$Executable;ExpectedSha256=$ExpectedSha256.ToUpperInvariant();ProbeArguments=$ProbeArguments;Libraries=$libraries}
    $configPath=Join-Path $RunDirectory 'run-spec.json';Write-RemovalJsonOnce $configPath $config
    $worker=Join-Path $RunDirectory 'worker.ps1'
    $task=New-RemovalTaskSpec -WorkerPath $worker -WorkerSha256 (Get-FileHash -LiteralPath $worker -Algorithm SHA256).Hash `
        -RunDirectory $RunDirectory -SessionId $SessionId -Nonce $Nonce -RunSpecSha256 (Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash
    [pscustomobject]@{RunDirectory=$RunDirectory;Config=$config;ControllerPid=$controller.Id
        ControllerCreationUtcTicks=$controller.StartTime.ToUniversalTime().Ticks;TaskOwner=(New-RemovalTaskOwner -Spec $task)
        ProbeOwner=$null;Error=$null;Terminal=$null;LastResult=$null}
}

function Observe-RemovalManagedSession {
    param([Parameter(Mandatory=$true,Position=0)]$Session)
    if($null -ne $Session.Terminal){return $Session.Terminal}
    $r=[ordered]@{Terminal=$false;NeedsObservation=$true;NativePassed=$false;Qualified=$false;Error=$Session.Error}
    $self=[Diagnostics.Process]::GetCurrentProcess()
    if($self.Id -ne $Session.ControllerPid -or $self.StartTime.ToUniversalTime().Ticks -ne $Session.ControllerCreationUtcTicks){throw 'Managed session belongs to another live controller.'}
    $o=$Session.TaskOwner;$root=$Session.RunDirectory
    # Even failed initial bindings retain a process. Observe those original
    # objects before any receipt/admission operation can throw again.
    $retainedWorker=Observe-RemovalRetainedWorker $o
    $retainedProbe=$null
    if($null -ne $Session.ProbeOwner){$retainedProbe=Observe-RemovalProbe $Session.ProbeOwner}
    try {
        $workerPath=Join-Path $root 'worker-start.json'
        if(-not $Session.Error -and -not $o.WorkerBindingKnown -and (Test-Path -LiteralPath $workerPath)){
            Attach-RemovalWorkerProcess $o (Read-RemovalJsonBounded $workerPath)
        }
        $readyPath=Join-Path $root 'probe-ready.json'
        if(-not $Session.Error -and $o.WorkerBindingKnown -and $null -eq $Session.ProbeOwner -and (Test-Path -LiteralPath $readyPath)){
            $s=New-RemovalProbeSpec -Executable $Session.Config.Executable -ExpectedSha256 $Session.Config.ExpectedSha256 `
                -RunDirectory $root -Nonce $o.Spec.Nonce -SessionId $o.Spec.SessionId -Sid $o.Spec.Sid `
                -ControllerPid $Session.ControllerPid -ControllerCreationUtcTicks $Session.ControllerCreationUtcTicks `
                -WorkerPid $o.WorkerPid -WorkerCreationUtcTicks $o.WorkerCreationUtcTicks -ProbeArguments $Session.Config.ProbeArguments
            $Session.ProbeOwner=New-RemovalProbeOwner -Spec $s
            Attach-RemovalProbeProcess $Session.ProbeOwner (Read-RemovalJsonBounded $readyPath)
        }
        if(-not $Session.Error -and $null -ne $Session.ProbeOwner -and $Session.ProbeOwner.BindingKnown -and -not $Session.ProbeOwner.AdmissionAttempted){
            Admit-RemovalProbe $Session.ProbeOwner
        }
        # Process liveness never depends on successful worker JSON publication.
        # A crash/write failure leaves these exact owners observable each tick.
        $state=$null
        if($o.WorkerBindingKnown){$state=Get-RemovalTaskObservation $o}
        if($null -ne $Session.ProbeOwner -and $null -eq $retainedProbe){$retainedProbe=Observe-RemovalProbe $Session.ProbeOwner}
        $resultPath=Join-Path $root 'worker-result.json'
        if($o.WorkerBindingKnown -and $null -ne $Session.ProbeOwner -and (Test-Path -LiteralPath $resultPath)){
            if($null -eq $Session.LastResult){$Session.LastResult=Read-RemovalJsonBounded $resultPath}
            $r=Complete-RemovalProbeTerminal $o $state $Session.ProbeOwner $Session.LastResult
            if($r.Terminal){$Session.Terminal=$r}
        }
    } catch {$Session.Error='Managed observation/admission failed/unknown: '+$_.Exception.Message;$r.Error=$Session.Error}
    return $r
}
