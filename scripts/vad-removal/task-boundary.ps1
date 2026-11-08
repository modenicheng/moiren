#requires -Version 5.1
# Definitions only. Caller must retain the owner before any scheduler mutation.
Set-StrictMode -Version 2

function Assert-RemovalTaskHost {
    if([Environment]::MachineName -cne 'LAPTOP-NP49RG6H' -or
        [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -cne 'S-1-5-21-3080296863-1242338503-1346362208-1001'){
        throw 'Wrong scheduler target host/SID.'
    }
}

function Assert-RemovalTaskNativeApproval {
    # This preparation has no reviewed worker/probe handshake yet. Mock tests
    # explicitly replace this guard; a boolean on Spec cannot authorize native work.
    throw 'Task preparation is not approved for native registration or launch.'
}

function Assert-RemovalTaskPath([string]$Path) {
    if($Path -notmatch '\A[A-Za-z]:\\' -or $Path -match '[\x00-\x1f"*?]' -or
        $Path.Substring(2).Contains(':') -or [IO.Path]::GetFullPath($Path) -cne $Path -or $Path.EndsWith('\')){
        throw 'Canonical task path without trailing separator required.'
    }
    $cursor=$Path
    while($cursor){
        if(Test-Path -LiteralPath $cursor){
            $item=Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
            if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Reparse task path refused.'}
            if($cursor -cne $Path -and -not $item.PSIsContainer){throw 'Non-directory task ancestor.'}
        }
        $cursor=[IO.Path]::GetDirectoryName($cursor)
    }
}

function New-RemovalTaskSpec {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$WorkerPath,
        [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$WorkerSha256,
        [Parameter(Mandatory=$true)][string]$RunDirectory,
        [Parameter(Mandatory=$true)][ValidateRange(1,65535)][int]$SessionId,
        [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{32}$')][string]$Nonce,
        [ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$RunSpecSha256)
    foreach($path in @($WorkerPath,$RunDirectory)){Assert-RemovalTaskPath $path}
    if([IO.Path]::GetDirectoryName($WorkerPath) -cne $RunDirectory -or -not(Test-Path -LiteralPath $WorkerPath -PathType Leaf) -or
        (Get-FileHash -LiteralPath $WorkerPath -Algorithm SHA256).Hash -ine $WorkerSha256){throw 'Pinned worker must reside in exact run directory.'}
    $name='Moiren-Removal613-'+$Nonce
    $exe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    Assert-RemovalTaskPath $exe
    $argv=@('-NoLogo','-NoProfile','-NonInteractive','-WindowStyle','Hidden','-File',$WorkerPath,
        '-TaskPath',('\'+$name),'-Nonce',$Nonce,'-ExpectedSessionId',[string]$SessionId,
        '-RunDirectory',$RunDirectory,'-WorkerSha256',$WorkerSha256)
    if($RunSpecSha256){$argv+=@('-RunSpecSha256',$RunSpecSha256.ToUpperInvariant())}
    # Validated task paths contain neither quotes nor trailing separators.
    $line=($argv|ForEach-Object{'"'+$_+'"'}) -join ' '
    [pscustomobject]@{Name=$name;Path=('\'+$name);Sid='S-1-5-21-3080296863-1242338503-1346362208-1001'
        SessionId=$SessionId;Nonce=$Nonce;WorkerPath=$WorkerPath;WorkerSha256=$WorkerSha256.ToUpperInvariant()
        Executable=$exe;ExecutableSha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
        RunDirectory=$RunDirectory;Arguments=$line;ReviewedForNative=$false}
}

function Assert-RemovalTaskDefinition($Spec,$Task) {
    $d=$Task.Definition
    if($Task.Path -cne $Spec.Path -or $Task.Name -cne $Spec.Name -or $d.Principal.UserId -cne $Spec.Sid -or
        $d.Principal.LogonType -ne 3 -or $d.Principal.RunLevel -ne 0 -or $d.Triggers.Count -ne 0 -or $d.Actions.Count -ne 1){
        throw 'Task identity/principal/action count/trigger differs.'
    }
    $a=$d.Actions.Item(1);$s=$d.Settings
    if($a.Type -ne 0 -or $a.Path -cne $Spec.Executable -or $a.Arguments -cne $Spec.Arguments -or
        $a.WorkingDirectory -cne $Spec.RunDirectory -or -not $Task.Enabled -or -not $s.Enabled -or -not $s.AllowDemandStart -or
        $s.MultipleInstances -ne 2 -or $s.RestartCount -ne 0 -or $s.AllowHardTerminate -or $s.ExecutionTimeLimit -cne 'PT0S' -or
        $s.StopIfGoingOnBatteries -or $s.DisallowStartIfOnBatteries){throw 'Task action/one-shot/nontermination policy differs.'}
}

function New-RemovalTaskOwner {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$Spec,$RegisteredTask=$null)
    if($null -ne $RegisteredTask){Assert-RemovalTaskDefinition $Spec $RegisteredTask}
    [pscustomobject]@{Spec=$Spec;Scheduler=$null;Folder=$null;TaskDefinition=$null
        RegistrationAttempted=($null -ne $RegisteredTask);RegistrationKnown=($null -ne $RegisteredTask)
        RegistrationUtc=$null;RegisteredTask=$RegisteredTask
        RegisteredXml=$(if($null -ne $RegisteredTask){$RegisteredTask.Xml}else{$null})
        DispatchAttempted=$false;RunningTask=$null;InstanceGuid=$null;WorkerProcess=$null;WorkerReceipt=$null
        WorkerPid=$null;WorkerCreationUtcTicks=$null
        WorkerBindingKnown=$false;WorkerBindingError=$null;LastObservation=$null
        DispatchUtc=$null;Error=$null;TerminalObserved=$false;Closed=$false}
}

function New-RemovalTaskScheduler {
    New-Object -ComObject 'Schedule.Service'
}

function Register-RemovalTaskDefinition {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$Owner)
    $ErrorActionPreference='Stop';Assert-RemovalTaskHost;Assert-RemovalTaskNativeApproval
    if($Owner.RegistrationAttempted -or $Owner.Closed -or $Owner.Error){throw 'Registration already attempted/refused; retain exact intent, never retry.'}
    $s=$Owner.Spec
    try {
        $Owner.Scheduler=New-RemovalTaskScheduler
        $Owner.Scheduler.Connect()
        $Owner.Folder=$Owner.Scheduler.GetFolder('\')
        $Owner.TaskDefinition=$Owner.Scheduler.NewTask(0)
        $d=$Owner.TaskDefinition
        $d.RegistrationInfo.Description='One exact removal613 diagnostic worker; no automatic native approval'
        $d.Principal.UserId=$s.Sid;$d.Principal.LogonType=3;$d.Principal.RunLevel=0
        $d.Settings.AllowDemandStart=$true;$d.Settings.Enabled=$true;$d.Settings.MultipleInstances=2
        $d.Settings.RestartCount=0;$d.Settings.AllowHardTerminate=$false;$d.Settings.ExecutionTimeLimit='PT0S'
        $d.Settings.StopIfGoingOnBatteries=$false;$d.Settings.DisallowStartIfOnBatteries=$false
        $a=$d.Actions.Create(0);$a.Path=$s.Executable;$a.Arguments=$s.Arguments;$a.WorkingDirectory=$s.RunDirectory
        # Retain intent and COM owners before TASK_CREATE can commit. A lost
        # return is an unknown registration, never permission to create again.
        $Owner.RegistrationUtc=[DateTimeOffset]::UtcNow.ToString('o')
        $Owner.RegistrationAttempted=$true
        $Owner.RegisteredTask=$Owner.Folder.RegisterTaskDefinition($s.Name,$d,2,$s.Sid,$null,3,$null)
        Assert-RemovalTaskDefinition $s $Owner.RegisteredTask
        $Owner.RegisteredXml=$Owner.RegisteredTask.Xml
        $Owner.RegistrationKnown=$true
    } catch {$Owner.Error='Task registration failed/unknown: '+$_.Exception.Message}
}

function Start-RemovalTaskOnce($Owner) {
    $ErrorActionPreference='Stop';Assert-RemovalTaskHost;Assert-RemovalTaskNativeApproval
    if(-not $Owner.RegistrationKnown -or $Owner.Error -or $Owner.Closed){throw 'No verified exact task registration.'}
    if($Owner.DispatchAttempted){throw 'Dispatch already attempted; observe exact task, never rerun.'}
    Assert-RemovalTaskDefinition $Owner.Spec $Owner.RegisteredTask
    if($Owner.RegisteredTask.Xml -cne $Owner.RegisteredXml -or $Owner.RegisteredTask.State -ne 3 -or
        $Owner.RegisteredTask.GetInstances(0).Count -ne 0 -or $Owner.RegisteredTask.LastTaskResult -ne 267011){throw 'Task changed/busy/already run.'}
    foreach($path in @($Owner.Spec.WorkerPath,$Owner.Spec.Executable,$Owner.Spec.RunDirectory)){Assert-RemovalTaskPath $path}
    if((Get-FileHash -LiteralPath $Owner.Spec.WorkerPath).Hash -cne $Owner.Spec.WorkerSha256 -or
        (Get-FileHash -LiteralPath $Owner.Spec.Executable).Hash -cne $Owner.Spec.ExecutableSha256){throw 'Task entry executable/script changed.'}
    $Owner.DispatchAttempted=$true;$Owner.DispatchUtc=[DateTimeOffset]::UtcNow.ToString('o')
    try {
        $Owner.RunningTask=$Owner.RegisteredTask.RunEx($null,4,$Owner.Spec.SessionId,$null)
        if($null -eq $Owner.RunningTask -or $Owner.RunningTask.Path -cne $Owner.Spec.Path -or
            $Owner.RunningTask.InstanceGuid -notmatch '^\{[a-fA-F0-9-]{36}\}$'){throw 'Running task instance unavailable.'}
        $Owner.InstanceGuid=$Owner.RunningTask.InstanceGuid
        # EnginePID is the scheduler engine, not proof of action/worker identity.
    } catch {$Owner.Error='Task dispatch failed/unknown; retain task and do not rerun.'}
}

function Assert-RemovalWorkerReceipt($Owner,$Receipt) {
    $s=$Owner.Spec
    if($null -eq $Receipt -or $Receipt.Nonce -cne $s.Nonce -or $Receipt.TaskPath -cne $s.Path -or
        $Receipt.Host -cne 'LAPTOP-NP49RG6H' -or $Receipt.Sid -cne $s.Sid -or $Receipt.SessionId -ne $s.SessionId -or
        $Receipt.IsAdministrator -isnot [bool] -or $Receipt.IsAdministrator -or $Receipt.WorkerSha256 -cne $s.WorkerSha256 -or
        ($Receipt.Pid -isnot [int] -and $Receipt.Pid -isnot [uint32] -and $Receipt.Pid -isnot [long]) -or $Receipt.Pid -le 0 -or
        $Receipt.CreationUtcTicks -isnot [long] -or $Receipt.CreationUtcTicks -le 0){throw 'Worker receipt identity/token/provenance differs.'}
}

function Open-RemovalWorkerProcess([int]$WorkerPid) {
    [Diagnostics.Process]::GetProcessById($WorkerPid)
}

function Read-RemovalWorkerProcessMetadata([Diagnostics.Process]$Process) {
    $ErrorActionPreference='Stop'
    $rows=@(Get-CimInstance -ClassName Win32_Process -Filter ('ProcessId='+$Process.Id) -OperationTimeoutSec 10)
    if($rows.Count -ne 1){throw 'Exact live worker metadata unavailable.'}
    $token=Read-RemovalWorkerToken $Process
    @{Pid=$rows[0].ProcessId;Sid=$token.Sid;IsAdministrator=$token.IsAdministrator;IsElevated=$token.IsElevated
        Executable=$rows[0].ExecutablePath;CommandLine=$rows[0].CommandLine}
}

function Read-RemovalWorkerToken([Diagnostics.Process]$Process) {
    if(-not ('MoirenRemoval613.WorkerToken' -as [type])){
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
namespace MoirenRemoval613 {
    public sealed class WorkerToken {
        public string Sid;
        public bool IsAdministrator;
        public bool IsElevated;
        [DllImport("advapi32.dll", SetLastError=true)]
        static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
        [DllImport("advapi32.dll", SetLastError=true)]
        static extern bool GetTokenInformation(IntPtr token, int kind, out int data, int length, out int returned);
        [DllImport("kernel32.dll", SetLastError=true)]
        static extern bool CloseHandle(IntPtr handle);
        public static WorkerToken Read(IntPtr process) {
            IntPtr token;
            // WindowsPrincipal duplicates the queried primary token for its
            // membership check; QUERY alone makes that read fail with access denied.
            if (!OpenProcessToken(process, 0xa, out token))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            try {
                int elevated, returned;
                if (!GetTokenInformation(token, 20, out elevated, 4, out returned))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                if (returned != 4 || (elevated != 0 && elevated != 1))
                    throw new InvalidOperationException("Unknown token elevation data.");
                using (var identity = new WindowsIdentity(token)) {
                    return new WorkerToken {
                        Sid = identity.User.Value,
                        IsAdministrator = new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator),
                        IsElevated = elevated != 0
                    };
                }
            } finally {
                CloseHandle(token);
            }
        }
    }
}
'@
    }
    # Query the token through the retained process handle, not a second PID
    # lookup or the worker's self-reported privilege field.
    [MoirenRemoval613.WorkerToken]::Read($Process.Handle)
}

function Attach-RemovalWorkerProcess {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$Owner,[Parameter(Mandatory=$true)]$Receipt)
    $ErrorActionPreference='Stop';Assert-RemovalTaskHost
    if(-not $Owner.RegistrationKnown -or -not $Owner.DispatchAttempted -or $Owner.Closed -or
        $null -ne $Owner.WorkerProcess -or $Owner.WorkerBindingError){throw 'No fresh worker binding is permitted.'}
    Assert-RemovalWorkerReceipt $Owner $Receipt
    $Owner.WorkerReceipt=$Receipt
    try {
        # Keep the first Process object before any handle/property/CIM read can
        # fail. A later observation uses this object, never a fresh PID lookup.
        $Owner.WorkerProcess=Open-RemovalWorkerProcess -WorkerPid $Receipt.Pid
        $process=$Owner.WorkerProcess
        $null=$process.Handle
        $process.Refresh()
        $dispatch=[DateTimeOffset]::Parse($Owner.DispatchUtc,[Globalization.CultureInfo]::InvariantCulture)
        $creation=$process.StartTime.ToUniversalTime()
        if($process.HasExited -or $process.Id -ne $Receipt.Pid -or
            $creation.Ticks -ne $Receipt.CreationUtcTicks -or $creation -lt $dispatch.UtcDateTime.AddSeconds(-2) -or
            $creation -gt [DateTime]::UtcNow -or
            $process.SessionId -ne $Owner.Spec.SessionId -or
            $process.MainModule.FileName -cne $Owner.Spec.Executable){throw 'Worker handle/creation/session/image differs.'}
        $metadata=Read-RemovalWorkerProcessMetadata $process
        $unquoted=$Owner.Spec.Executable+' '+$Owner.Spec.Arguments
        $quoted='"'+$Owner.Spec.Executable+'" '+$Owner.Spec.Arguments
        if($metadata.Pid -ne $Receipt.Pid -or $metadata.Sid -cne $Owner.Spec.Sid -or
            $metadata.IsAdministrator -isnot [bool] -or $metadata.IsAdministrator -or
            $metadata.IsElevated -isnot [bool] -or $metadata.IsElevated -or
            $metadata.Executable -cne $Owner.Spec.Executable -or
            ($metadata.CommandLine -cne $quoted -and $metadata.CommandLine -cne $unquoted) -or
            (Get-FileHash -LiteralPath $Owner.Spec.WorkerPath -Algorithm SHA256).Hash -cne $Owner.Spec.WorkerSha256 -or
            (Get-FileHash -LiteralPath $Owner.Spec.Executable -Algorithm SHA256).Hash -cne $Owner.Spec.ExecutableSha256){
            throw 'Worker command line/SID/on-disk provenance differs.'
        }
        $process.Refresh()
        if($process.HasExited){throw 'Worker exited during initial binding; no launch admission.'}
        $Owner.WorkerPid=[int]$Receipt.Pid
        $Owner.WorkerCreationUtcTicks=[long]$Receipt.CreationUtcTicks
        $Owner.WorkerBindingKnown=$true
    } catch {
        $Owner.WorkerBindingError='Worker binding failed/unknown: '+$_.Exception.Message
        throw
    }
}

function Get-RemovalTaskObservation($Owner) {
    $ErrorActionPreference='Stop';Assert-RemovalTaskHost
    $r=[ordered]@{State=$null;Instances=$null;LastTaskResult=$null;LastRunKnown=$false;ObservedUtc=$null;Error=$null}
    $Owner.LastObservation=$r
    try {
        if(-not $Owner.RegistrationKnown -or $null -eq $Owner.RegisteredTask){throw 'Unknown registration.'}
        Assert-RemovalTaskDefinition $Owner.Spec $Owner.RegisteredTask
        if($Owner.RegisteredTask.Xml -cne $Owner.RegisteredXml){throw 'Registered task definition changed.'}
        $r.State=[int]$Owner.RegisteredTask.State
        $r.Instances=[int]$Owner.RegisteredTask.GetInstances(0).Count
        $r.LastTaskResult=[int]$Owner.RegisteredTask.LastTaskResult
        $last=$Owner.RegisteredTask.LastRunTime
        $now=[DateTimeOffset]::UtcNow
        $r.ObservedUtc=$now.ToString('o')
        if($Owner.DispatchAttempted -and $null -ne $Owner.InstanceGuid -and $last -is [DateTime]){
            $dispatch=[DateTimeOffset]::Parse($Owner.DispatchUtc,[Globalization.CultureInfo]::InvariantCulture)
            $lastUtc=$last.ToUniversalTime()
            # Scheduler dates have second precision. The nonce-named task was
            # verified never-run before this one retained RunEx invocation.
            $r.LastRunKnown=($lastUtc -ge $dispatch.UtcDateTime.AddSeconds(-2) -and
                $lastUtc -le $now.UtcDateTime.AddSeconds(2) -and $r.LastTaskResult -in @(0,1))
        }
    } catch {$r.Error=$_.Exception.Message}
    return $r
}

function Test-RemovalTaskTerminal($Owner,$State,$WorkerResult) {
    $ErrorActionPreference='Stop'
    $r=[ordered]@{Terminal=$false;NeedsObservation=$true;WorkerTerminal=$false;WorkerNeedsObservation=$true
        ProbeOwnershipVerified=$false;ReportedProbeExitCode=$null;TaskExitCode=$State.LastTaskResult;NativePassed=$false;Qualified=$false}
    if(-not [object]::ReferenceEquals($State,$Owner.LastObservation) -or $State.Error -or
        -not $Owner.DispatchAttempted -or -not $Owner.WorkerBindingKnown -or $Owner.WorkerBindingError -or
        $null -eq $Owner.WorkerProcess -or $State.LastRunKnown -isnot [bool] -or -not $State.LastRunKnown -or
        $State.State -notin @(1,3) -or $State.Instances -ne 0 -or
        ($State.LastTaskResult -isnot [int] -and $State.LastTaskResult -isnot [long]) -or $State.LastTaskResult -notin @(0,1) -or $null -eq $WorkerResult){return $r}
    Assert-RemovalWorkerReceipt $Owner $WorkerResult
    if($WorkerResult.Pid -ne $Owner.WorkerPid -or
        $WorkerResult.CreationUtcTicks -ne $Owner.WorkerCreationUtcTicks -or
        $WorkerResult.ProbeExitKnown -isnot [bool] -or -not $WorkerResult.ProbeExitKnown -or
        ($WorkerResult.ProbeExitCode -isnot [int] -and $WorkerResult.ProbeExitCode -isnot [long]) -or
        $WorkerResult.Owners -isnot [Array] -or $WorkerResult.Owners.Count -ne 0 -or
        ($WorkerResult.WorkerExitCode -isnot [int] -and $WorkerResult.WorkerExitCode -isnot [long]) -or
        $WorkerResult.WorkerExitCode -ne $State.LastTaskResult){return $r}
    $Owner.WorkerProcess.Refresh()
    if(-not $Owner.WorkerProcess.HasExited -or $Owner.WorkerProcess.ExitCode -ne $WorkerResult.WorkerExitCode){return $r}
    # This boundary proves only task/worker exit. The actual native child owner
    # handshake is still missing, so never promote its detached reported exit
    # to full diagnostic Terminal/no-owners (the controller's admission gate).
    $r.WorkerTerminal=$true;$r.WorkerNeedsObservation=$false
    $r.ReportedProbeExitCode=$WorkerResult.ProbeExitCode
    return $r
}

function Observe-RemovalRetainedWorker($Owner) {
    $r=[ordered]@{ExitKnown=$false;ExitCode=$null;NeedsObservation=$true;Error=$null;NativePassed=$false}
    if($null -eq $Owner.WorkerProcess){return $r}
    try {
        # Also observe a failed initial binding through its retained Process.
        # This proves liveness only, never retroactive admission or native pass.
        $Owner.WorkerProcess.Refresh()
        if($Owner.WorkerProcess.HasExited){
            $r.ExitKnown=$true;$r.ExitCode=[int]$Owner.WorkerProcess.ExitCode;$r.NeedsObservation=$false
        }
    } catch {$r.Error='Retained worker observation unavailable.'}
    return $r
}
