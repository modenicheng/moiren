#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'worker-boundary.ps1')
function Assert-RemovalTaskHost {}
function Assert-RemovalTaskNativeApproval {}
function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
$root=Join-Path $PSScriptRoot ('task-fixtures\worker-process-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$entry=Join-Path $root 'worker.ps1';$probe=Join-Path $root 'probe.ps1'
$bootstrap=@'
param([string]$TaskPath,[string]$Nonce,[int]$ExpectedSessionId,[string]$RunDirectory,[string]$WorkerSha256)
$ErrorActionPreference='Stop'
$config=Get-Content -LiteralPath (Join-Path $RunDirectory 'fixture.json') -Raw|ConvertFrom-Json
. $config.Library
function Assert-RemovalTaskHost {}
$p=[Diagnostics.Process]::GetCurrentProcess()
$s=New-RemovalProbeSpec -Executable $config.Executable -ExpectedSha256 $config.ExecutableSha256 -RunDirectory $RunDirectory -Nonce $Nonce -SessionId $ExpectedSessionId -Sid $config.Sid -ControllerPid $config.ControllerPid -ControllerCreationUtcTicks $config.ControllerCreationUtcTicks -WorkerPid $p.Id -WorkerCreationUtcTicks $p.StartTime.ToUniversalTime().Ticks -ProbeArguments @('live-vad','--seconds','8')
# This harmless test script replaces only the native executable entry, not the
# real worker ownership, output drains, controller binding or terminal checks.
$argv=@('-NoLogo','-NoProfile','-NonInteractive','-File',$config.Probe,'-RunDirectory',$RunDirectory,'-Nonce',$Nonce,'-ControllerPid',[string]$s.ControllerPid,'-ControllerTicks',[string]$s.ControllerCreationUtcTicks,'-WorkerPid',[string]$p.Id,'-WorkerTicks',[string]$s.WorkerCreationUtcTicks)
$s.CommandLine=($argv|ForEach-Object{Quote-FiniteToolArgument $_}) -join ' '
$s.Arguments=$argv
$result=Invoke-RemovalWorker -Spec $s -TaskPath $TaskPath -WorkerPath $PSCommandPath -WorkerSha256 $WorkerSha256
exit $result.WorkerExitCode
'@
$probeText=@'
param([string]$RunDirectory,[string]$Nonce,[int]$ControllerPid,[long]$ControllerTicks,[int]$WorkerPid,[long]$WorkerTicks)
$ErrorActionPreference='Stop'
$p=[Diagnostics.Process]::GetCurrentProcess()
$controller=[Diagnostics.Process]::GetProcessById($ControllerPid);$null=$controller.Handle
$worker=[Diagnostics.Process]::GetProcessById($WorkerPid);$null=$worker.Handle
if($controller.StartTime.ToUniversalTime().Ticks-ne$ControllerTicks-or$worker.StartTime.ToUniversalTime().Ticks-ne$WorkerTicks){exit 2}
$r=@{schema_version=1;nonce=$Nonce;probe_pid=$PID;probe_creation_utc_ticks=$p.StartTime.ToUniversalTime().Ticks;controller_pid=$ControllerPid;controller_creation_utc_ticks=$ControllerTicks;worker_pid=$WorkerPid;worker_creation_utc_ticks=$WorkerTicks;session_id=$p.SessionId}
$ready=Join-Path $RunDirectory 'probe-ready.json'
[IO.File]::WriteAllText(($ready+'.partial'),($r|ConvertTo-Json -Compress),[Text.UTF8Encoding]::new($false));[IO.File]::Move(($ready+'.partial'),$ready)
$clock=[Diagnostics.Stopwatch]::StartNew();$ack=Join-Path $RunDirectory 'probe-admit.json'
while(-not(Test-Path -LiteralPath $ack)){
 if($clock.ElapsedMilliseconds-ge15000-or$controller.HasExited-or$worker.HasExited){exit 2}
 Start-Sleep -Milliseconds 10
}
$actual=Get-Content -LiteralPath $ack -Raw|ConvertFrom-Json
foreach($key in $r.Keys){if($actual.$key-cne$r[$key]){exit 2}}
Write-Output 'harmless probe admitted; no audio'
exit 1
'@
[IO.File]::WriteAllText($entry,$bootstrap,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($probe,$probeText,[Text.UTF8Encoding]::new($false))
$self=[Diagnostics.Process]::GetCurrentProcess();$exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
Check ($self.SessionId-gt0-and-not([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) 'Run this host-only test from a non-elevated nonzero session.'
$taskSpec=New-RemovalTaskSpec -WorkerPath $entry -WorkerSha256 (Get-FileHash -LiteralPath $entry).Hash -RunDirectory $root -SessionId $self.SessionId -Nonce ([Guid]::NewGuid().ToString('N'))
$taskSpec.Sid=$sid
$config=@{Library=(Join-Path $PSScriptRoot 'worker-boundary.ps1');Executable=$exe;ExecutableSha256=(Get-FileHash -LiteralPath $exe).Hash
 Sid=$sid;ControllerPid=$PID;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks;Probe=$probe}
Write-RemovalJsonOnce (Join-Path $root 'fixture.json') $config
$task=[pscustomobject]@{Path=$taskSpec.Path;Name=$taskSpec.Name;Enabled=$true;State=3;LastTaskResult=267011;LastRunTime=[DateTime]'1899-12-30';Xml='<mock-scheduler/>'
 Definition=[pscustomobject]@{Principal=[pscustomobject]@{UserId=$sid;LogonType=3;RunLevel=0};Triggers=[pscustomobject]@{Count=0}
 Actions=[pscustomobject]@{Count=1;Action=[pscustomobject]@{Type=0;Path=$exe;Arguments=$taskSpec.Arguments;WorkingDirectory=$root}}
 Settings=[pscustomobject]@{Enabled=$true;AllowDemandStart=$true;MultipleInstances=2;RestartCount=0;AllowHardTerminate=$false;ExecutionTimeLimit='PT0S';StopIfGoingOnBatteries=$false;DisallowStartIfOnBatteries=$false}}}
$task.Definition.Actions|Add-Member ScriptMethod Item {param($i)return $this.Action}
$task|Add-Member ScriptMethod GetInstances {param($flags)return [pscustomobject]@{Count=0}}
$task|Add-Member ScriptMethod RunEx {param($parameters,$flags,$session,$user)return [pscustomobject]@{Path=$this.Path;InstanceGuid='{d5d7acdf-e49d-4134-ac20-3d965accb18c}'}}
$taskOwner=New-RemovalTaskOwner -Spec $taskSpec -RegisteredTask $task
$worker=[Diagnostics.Process]::new();$probeOwner=$null;$checks=0
$worker.StartInfo=[Diagnostics.ProcessStartInfo]::new();$worker.StartInfo.FileName=$exe;$worker.StartInfo.Arguments=$taskSpec.Arguments
$worker.StartInfo.UseShellExecute=$false;$worker.StartInfo.CreateNoWindow=$true;$worker.StartInfo.WorkingDirectory=$root
$worker.StartInfo.RedirectStandardOutput=$true;$worker.StartInfo.RedirectStandardError=$true
$worker.StartInfo.EnvironmentVariables['PSModulePath']=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules'
function AwaitReceipt([string]$Name) {
 $path=Join-Path $root $Name;$clock=[Diagnostics.Stopwatch]::StartNew()
 while(-not(Test-Path -LiteralPath $path)){
  if($worker.HasExited-or$clock.ElapsedMilliseconds-ge10000){throw ('Missing receipt '+$Name)}
  Start-Sleep -Milliseconds 10
 }
 Read-RemovalJsonBounded $path
}
try {
 Start-RemovalTaskOnce $taskOwner;Check ($worker.Start()) 'Real worker not dispatched';$null=$worker.Handle
 $stdout=$worker.StandardOutput.ReadToEndAsync();$stderr=$worker.StandardError.ReadToEndAsync()
 $start=AwaitReceipt 'worker-start.json';$start.Host='LAPTOP-NP49RG6H'
 Attach-RemovalWorkerProcess $taskOwner $start;$checks++
 $s=New-RemovalProbeSpec -Executable $exe -ExpectedSha256 $config.ExecutableSha256 -RunDirectory $root -Nonce $taskSpec.Nonce -SessionId $self.SessionId -Sid $sid -ControllerPid $PID -ControllerCreationUtcTicks $self.StartTime.ToUniversalTime().Ticks -WorkerPid $worker.Id -WorkerCreationUtcTicks $worker.StartTime.ToUniversalTime().Ticks -ProbeArguments @('live-vad','--seconds','8')
 $argv=@('-NoLogo','-NoProfile','-NonInteractive','-File',$probe,'-RunDirectory',$root,'-Nonce',$taskSpec.Nonce,'-ControllerPid',[string]$PID,'-ControllerTicks',[string]$s.ControllerCreationUtcTicks,'-WorkerPid',[string]$worker.Id,'-WorkerTicks',[string]$s.WorkerCreationUtcTicks)
 $s.CommandLine=($argv|ForEach-Object{Quote-FiniteToolArgument $_}) -join ' ';$s.Arguments=$argv
 $probeOwner=New-RemovalProbeOwner $s;$ready=AwaitReceipt 'probe-ready.json';Attach-RemovalProbeProcess $probeOwner $ready;$checks++
 Check (-not(Test-Path -LiteralPath (Join-Path $root 'worker-result.json'))-and-not$probeOwner.Process.HasExited) 'Child passed gate without controller';$checks++
 Admit-RemovalProbe $probeOwner
 $result=AwaitReceipt 'worker-result.json';$result.Host='LAPTOP-NP49RG6H'
 Check ($worker.WaitForExit(15000)) 'Same harmless worker still running';$checks++
 $task.LastRunTime=[DateTime]::Now;$task.LastTaskResult=$worker.ExitCode
 $state=Get-RemovalTaskObservation $taskOwner
 $complete=Complete-RemovalProbeTerminal $taskOwner $state $probeOwner $result
 Check ($complete.Terminal-and$complete.ProbeOwnershipVerified-and$complete.ProbeExitCode-eq1-and-not$complete.NativePassed-and-not$complete.NeedsObservation) 'Full independent process exit chain incomplete';$checks++
 Check (([IO.File]::ReadAllText((Join-Path $root 'probe-stdout.log'))).Contains('no audio')) 'Probe pipe output missing';$checks++
 Write-Output ('CHECKS='+$checks+' FAILED=0 SCHEDULER=mock WORKER=real PROBE=real NATIVE_EXECUTED=false')
} catch {
 Write-Output ('CHECKS='+$checks+' FAILED=1 ERROR='+$_.Exception.Message)
 if(-not$worker.HasExited){$null=$worker.WaitForExit(20000)}
 if($worker.HasExited-and$stderr.IsCompleted){Write-Output $stderr.GetAwaiter().GetResult()}
 exit 1
} finally {
 if($null-ne$probeOwner-and$null-ne$probeOwner.Process-and$probeOwner.Process.HasExited){$probeOwner.Process.Dispose()}
 if($null-ne$taskOwner.WorkerProcess-and$taskOwner.WorkerProcess.HasExited){$taskOwner.WorkerProcess.Dispose()}
 if($worker.HasExited){$worker.Dispose()}
}
