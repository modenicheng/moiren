#requires -Version 5.1
param([string]$Library)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
if(-not $Library){$Library=Join-Path $PSScriptRoot 'task-boundary.ps1'}
. $Library

# Scheduler and target identity are fixtures. The child, token, command line,
# original handles, start time and natural exit below are real local reads.
function Assert-RemovalTaskHost {}
function Assert-RemovalTaskNativeApproval {}
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
$root=Join-Path $PSScriptRoot ('task-fixtures\process-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$workerPath=Join-Path $root 'worker.ps1'
$workerText=@'
param([string]$TaskPath,[string]$Nonce,[int]$ExpectedSessionId,[string]$RunDirectory,[string]$WorkerSha256)
$ErrorActionPreference='Stop'
$p=[Diagnostics.Process]::GetCurrentProcess()
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$r=@{TaskPath=$TaskPath;Nonce=$Nonce;Host='LAPTOP-NP49RG6H';Sid=$identity.User.Value
 SessionId=$p.SessionId;Pid=$PID;CreationUtcTicks=$p.StartTime.ToUniversalTime().Ticks
 IsAdministrator=([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
 WorkerSha256=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash}
[IO.File]::WriteAllText((Join-Path $RunDirectory 'started.tmp'),($r|ConvertTo-Json))
[IO.File]::Move((Join-Path $RunDirectory 'started.tmp'),(Join-Path $RunDirectory 'started.json'))
$clock=[Diagnostics.Stopwatch]::StartNew()
while(-not(Test-Path -LiteralPath (Join-Path $RunDirectory 'release.flag'))){
 if($clock.Elapsed.TotalSeconds -ge 12){exit 2}
 Start-Sleep -Milliseconds 20
}
exit 1
'@
[IO.File]::WriteAllText($workerPath,$workerText,[Text.UTF8Encoding]::new($false))
$session=[Diagnostics.Process]::GetCurrentProcess().SessionId
$s=New-RemovalTaskSpec -WorkerPath $workerPath -WorkerSha256 (Get-FileHash -LiteralPath $workerPath).Hash `
    -RunDirectory $root -SessionId $session -Nonce ([Guid]::NewGuid().ToString('N'))
$s.Sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$task=[pscustomobject]@{Path=$s.Path;Name=$s.Name;Enabled=$true;State=3;LastTaskResult=267011
    LastRunTime=[DateTime]'1899-12-30';Xml='<mock-scheduler-only/>';RunCalls=0
    Definition=[pscustomobject]@{Principal=[pscustomobject]@{UserId=$s.Sid;LogonType=3;RunLevel=0}
        Triggers=[pscustomobject]@{Count=0};Actions=[pscustomobject]@{Count=1
            Action=[pscustomobject]@{Type=0;Path=$s.Executable;Arguments=$s.Arguments;WorkingDirectory=$root}}
        Settings=[pscustomobject]@{Enabled=$true;AllowDemandStart=$true;MultipleInstances=2;RestartCount=0
            AllowHardTerminate=$false;ExecutionTimeLimit='PT0S';StopIfGoingOnBatteries=$false;DisallowStartIfOnBatteries=$false}}}
$task.Definition.Actions|Add-Member ScriptMethod Item {param($i)return $this.Action}
$task|Add-Member ScriptMethod GetInstances {param($flags)return [pscustomobject]@{Count=0}}
$task|Add-Member ScriptMethod RunEx {
    param($parameters,$flags,$session,$user)
    $this.RunCalls++
    return [pscustomobject]@{Path=$this.Path;InstanceGuid='{d5d7acdf-e49d-4134-ac20-3d965accb18c}'}
}
$owner=New-RemovalTaskOwner -Spec $s -RegisteredTask $task
$child=[Diagnostics.Process]::new()
$child.StartInfo=[Diagnostics.ProcessStartInfo]::new()
$child.StartInfo.FileName=$s.Executable
$child.StartInfo.Arguments=$s.Arguments
$child.StartInfo.WorkingDirectory=$root
$child.StartInfo.UseShellExecute=$false
$child.StartInfo.CreateNoWindow=$true
$child.StartInfo.RedirectStandardOutput=$true
$child.StartInfo.RedirectStandardError=$true
$child.StartInfo.EnvironmentVariables['PSModulePath']=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules'
$checks=0
try {
    Start-RemovalTaskOnce $owner
    Check ($owner.DispatchAttempted -and $null -eq $owner.Error) 'Mock task dispatch failed.'
    Check ($child.Start()) 'Harmless child did not start.'
    $null=$child.Handle
    $stdout=$child.StandardOutput.ReadToEndAsync()
    $stderr=$child.StandardError.ReadToEndAsync()
    $clock=[Diagnostics.Stopwatch]::StartNew()
    $started=Join-Path $root 'started.json'
    while(-not(Test-Path -LiteralPath $started)){
        if($child.HasExited -or $clock.Elapsed.TotalSeconds -ge 8){throw 'Child start receipt unavailable.'}
        Start-Sleep -Milliseconds 20
    }
    $receipt=Get-Content -LiteralPath $started -Raw|ConvertFrom-Json
    Attach-RemovalWorkerProcess -Owner $owner -Receipt $receipt
    Check ($owner.WorkerBindingKnown -and $owner.WorkerPid -eq $child.Id -and
        $owner.WorkerCreationUtcTicks -eq $child.StartTime.ToUniversalTime().Ticks) 'Real child binding failed.'
    $checks++
    $token=Read-RemovalWorkerToken $owner.WorkerProcess
    Check ($token.Sid -ceq $s.Sid -and -not $token.IsAdministrator -and -not $token.IsElevated) 'Real limited token differs.'
    $checks++
    $receipt|Add-Member NoteProperty ProbeExitKnown $true
    $receipt|Add-Member NoteProperty ProbeExitCode 1
    $receipt|Add-Member NoteProperty WorkerExitCode 1
    $receipt|Add-Member NoteProperty Owners @()
    $task.LastRunTime=[DateTime]::Now;$task.LastTaskResult=1
    $state=Get-RemovalTaskObservation $owner
    $r=Test-RemovalTaskTerminal $owner $state $receipt
    Check (-not $r.WorkerTerminal -and $r.WorkerNeedsObservation -and -not $r.Terminal) 'Live real worker was accepted as terminal.'
    $checks++
    [IO.File]::WriteAllText((Join-Path $root 'release.flag'),'natural exit requested')
    Check ($child.WaitForExit(15000)) 'Harmless child remains live; retain its original owner.'
    $state=Get-RemovalTaskObservation $owner
    $r=Test-RemovalTaskTerminal $owner $state $receipt
    Check ($r.WorkerTerminal -and -not $r.WorkerNeedsObservation -and $r.ReportedProbeExitCode -eq 1 -and
        -not $r.NativePassed -and $child.ExitCode -eq 1) 'Real natural-exit evidence differs.'
    Check (-not $r.Terminal -and $r.NeedsObservation -and -not $r.ProbeOwnershipVerified) 'Harmless worker promoted to native terminal.'
    $checks++
    Write-Output ('CHECKS='+$checks+' FAILED=0 NATIVE_EXECUTED=false SCHEDULER=mock CHILD=real')
} catch {
    Write-Output ('CHECKS='+$checks+' FAILED=1 ERROR='+$_.Exception.Message)
    if($null -ne $owner.WorkerProcess){Write-Output ('RETAINED_WORKER='+$owner.WorkerProcess.Id)}
    if($child.Id -gt 0 -and -not $child.HasExited){$null=$child.WaitForExit(15000)}
    if($child.HasExited -and $null -ne $stderr -and $stderr.IsCompleted){
        $errorText=$stderr.GetAwaiter().GetResult()
        Write-Output ('CHILD_STDERR='+$errorText.Substring(0,[Math]::Min(4096,$errorText.Length)))
    }
    exit 1
} finally {
    if($null -ne $owner.WorkerProcess -and $owner.WorkerProcess.HasExited){$owner.WorkerProcess.Dispose()}
    if($child.HasExited){$child.Dispose()}
}
