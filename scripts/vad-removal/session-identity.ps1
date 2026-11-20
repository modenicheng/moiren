#requires -Version 5.1
# Definitions only. Discovery is not worker admission or proof of audio access.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'task-boundary.ps1')

function Get-RemovalSessionExplorerPath {
    [IO.Path]::GetFullPath((Join-Path $env:SystemRoot 'explorer.exe'))
}

function Get-RemovalSessionProcessIds {
    $ErrorActionPreference='Stop'
    Get-CimInstance -ClassName Win32_Process -Filter "Name='explorer.exe'" -OperationTimeoutSec 5 |
        ForEach-Object {$_.ProcessId}
}

function Open-RemovalSessionProcess([int]$ProcessId) {
    [Diagnostics.Process]::GetProcessById($ProcessId)
}

function Read-RemovalSessionProcessMetadata($Process) {
    $ErrorActionPreference='Stop'
    # The caller retains this exact Process before any handle/property/token
    # read. Reuse the worker's primary-token reader; no WTS/SeTcb dependency.
    $handle=$Process.Handle
    if($handle -isnot [IntPtr] -or $handle -eq [IntPtr]::Zero){throw 'Missing original user-process handle.'}
    $Process.Refresh()
    if($Process.HasExited -isnot [bool] -or $Process.HasExited){throw 'User process is no longer live.'}
    $created=$Process.StartTime
    $session=$Process.SessionId
    $id=$Process.Id
    $image=$Process.MainModule.FileName
    $token=Read-RemovalWorkerToken $Process
    $Process.Refresh()
    if($Process.HasExited -or $Process.Id -ne $id -or $Process.SessionId -ne $session -or
        $Process.StartTime -ne $created){throw 'Original user process changed during observation.'}
    [pscustomobject][ordered]@{ProcessId=$id;CreationUtcTicks=$created.ToUniversalTime().Ticks
        SessionId=$session;ExecutablePath=$image;Sid=$token.Sid
        IsAdministrator=$token.IsAdministrator;IsElevated=$token.IsElevated}
}

function New-RemovalSessionIdentity {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][ValidatePattern('^S-1-5-21-[0-9-]+$')][string]$Sid,
        [ValidateRange(0,65535)][int]$WorkerSessionId=0)
    $path=Get-RemovalSessionExplorerPath
    $config=[ordered]@{Sid=$Sid;RequestedSessionId=$WorkerSessionId;ExplorerPath=$path}
    [pscustomobject]@{SchemaVersion=1;Sid=$Sid;RequestedSessionId=$WorkerSessionId;ExplorerPath=$path
        ConfigJson=(ConvertTo-Json -InputObject $config -Compress);Attempted=$false;Ready=$false;Error=$null
        Candidates=(New-Object Collections.Generic.List[object]);SelectedCandidates=@()
        Snapshot=$null;SnapshotJson=$null;ObservedUtc=$null;NativeApproved=$false;AudioAccessProven=$false}
}

function Assert-RemovalSessionCandidate($Owner,$Candidate,$Metadata) {
    foreach($name in @('ProcessId','SessionId','CreationUtcTicks')){
        $value=$Metadata.$name
        if(($value -isnot [int] -and $value -isnot [long] -and $value -isnot [uint32]) -or
            $value -lt 0){throw 'User-process identity requires typed nonnegative integers.'}
    }
    if($Metadata.ProcessId -ne $Candidate.ProcessId -or $Metadata.ProcessId -le 0 -or
        $Metadata.CreationUtcTicks -le 0 -or $Metadata.SessionId -gt 65535 -or
        $Metadata.ExecutablePath -isnot [string] -or $Metadata.ExecutablePath -ine $Owner.ExplorerPath -or
        $Metadata.Sid -isnot [string] -or $Metadata.Sid -notmatch '^S-1-5-21-[0-9-]+$' -or
        $Metadata.IsAdministrator -isnot [bool] -or $Metadata.IsElevated -isnot [bool]){
        throw 'Exact Explorer image/process/token identity is missing or malformed.'
    }
    if($Metadata.Sid -ceq $Owner.Sid -and ($Metadata.SessionId -eq 0 -or
        $Metadata.IsAdministrator -or $Metadata.IsElevated)){
        throw 'Target user requires a nonzero observed session and a Limited token.'
    }
}

function Assert-RemovalSessionConfiguration($Owner) {
    $config=[ordered]@{Sid=$Owner.Sid;RequestedSessionId=$Owner.RequestedSessionId;ExplorerPath=$Owner.ExplorerPath}
    if((ConvertTo-Json -InputObject $config -Compress) -cne $Owner.ConfigJson){
        throw 'Original session-discovery configuration changed.'
    }
}

function Read-RemovalSessionIdentity {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($Owner.Attempted){throw 'Discovery already attempted; revalidate retained candidates, never reopen by PID.'}
    $Owner.Attempted=$true
    try {
        Assert-RemovalTaskHost;Assert-RemovalSessionConfiguration $Owner
        $ids=@(Get-RemovalSessionProcessIds)
        if($ids.Count -eq 0 -or $ids.Count -gt 128){throw 'No bounded observed Explorer candidates; explicit observed user identity is required.'}
        $seen=@{}
        foreach($id in $ids){
            if(($id -isnot [int] -and $id -isnot [uint32] -and $id -isnot [long]) -or
                $id -le 0 -or $id -gt [int]::MaxValue -or $seen.ContainsKey([string]$id)){
                throw 'Malformed/duplicate Explorer process identity.'
            }
            $seen[[string]$id]=$true
            $candidate=[pscustomobject]@{ProcessId=[int]$id;Process=$null;RetainedProcess=$null
                Metadata=$null;IdentityJson=$null}
            $Owner.Candidates.Add($candidate)
            # Retain before Handle, MainModule or token access can fail. Never
            # stop/dispose a live foreign process or reacquire it after failure.
            $candidate.Process=Open-RemovalSessionProcess -ProcessId ([int]$id)
            $candidate.RetainedProcess=$candidate.Process
            $metadata=Read-RemovalSessionProcessMetadata $candidate.Process
            Assert-RemovalSessionCandidate $Owner $candidate $metadata
            $candidate.Metadata=$metadata
            $candidate.IdentityJson=ConvertTo-Json -InputObject $metadata -Compress
        }
        $matching=@($Owner.Candidates|Where-Object{$_.Metadata.Sid -ceq $Owner.Sid})
        if($Owner.RequestedSessionId -gt 0){
            $matching=@($matching|Where-Object{$_.Metadata.SessionId -eq $Owner.RequestedSessionId})
        }
        $sessions=@($matching|ForEach-Object{$_.Metadata.SessionId}|Sort-Object -Unique)
        if($sessions.Count -ne 1){throw 'Select one observed target-user session; no default session1 or ambiguous adoption.'}
        $Owner.SelectedCandidates=@($matching|Sort-Object ProcessId)
        $first=$Owner.SelectedCandidates[0].Metadata
        $now=[DateTimeOffset]::UtcNow.ToString('o')
        $Owner.Snapshot=[pscustomobject][ordered]@{SchemaVersion=1;Sid=$first.Sid;SessionId=$first.SessionId
            ProcessId=$first.ProcessId;CreationUtcTicks=$first.CreationUtcTicks;ExecutablePath=$first.ExecutablePath
            IsAdministrator=$first.IsAdministrator;IsElevated=$first.IsElevated;ObservedUtc=$now
            NativeApproved=$false;AudioAccessProven=$false}
        $Owner.SnapshotJson=ConvertTo-Json -InputObject $Owner.Snapshot -Compress
        $Owner.ObservedUtc=$now;$Owner.Ready=$true
        return $Owner.Snapshot
    } catch {$Owner.Ready=$false;$Owner.Error=$_.Exception.Message;throw}
}

function Assert-RemovalSessionIdentity {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    try {
        Assert-RemovalTaskHost;Assert-RemovalSessionConfiguration $Owner
        if(-not$Owner.Attempted -or -not$Owner.Ready -or $Owner.Error -or
            $Owner.SelectedCandidates.Count -eq 0 -or $null -eq $Owner.Snapshot -or
            (ConvertTo-Json -InputObject $Owner.Snapshot -Compress) -cne $Owner.SnapshotJson){
            throw 'Original selected session is missing, failed or changed.'
        }
        $selected=@($Owner.Candidates|Where-Object{$null -ne $_.Metadata -and
            $_.Metadata.Sid -ceq $Owner.Sid -and $_.Metadata.SessionId -eq $Owner.Snapshot.SessionId}|Sort-Object ProcessId)
        if($selected.Count -eq 0 -or $selected.Count -ne $Owner.SelectedCandidates.Count){
            throw 'Original selected candidate set changed.'
        }
        for($index=0;$index -lt $selected.Count;$index++){
            $candidate=$selected[$index]
            if(-not[Object]::ReferenceEquals($candidate,$Owner.SelectedCandidates[$index]) -or
                $null -eq $candidate.Process -or
                -not[Object]::ReferenceEquals($candidate.Process,$candidate.RetainedProcess)){
                throw 'Original user-process owner was replaced.'
            }
            $metadata=Read-RemovalSessionProcessMetadata $candidate.Process
            Assert-RemovalSessionCandidate $Owner $candidate $metadata
            if((ConvertTo-Json -InputObject $metadata -Compress) -cne $candidate.IdentityJson){
                throw 'Original process session/token/creation identity changed.'
            }
        }
        # This refresh concerns only the retained processes, not a fresh global
        # session inventory or desktop-unlock check. Scheduler/worker/probe
        # binding remains the final authority; no RT/audio call occurs here.
        $Owner.ObservedUtc=[DateTimeOffset]::UtcNow.ToString('o')
        $Owner.Snapshot.ObservedUtc=$Owner.ObservedUtc
        $Owner.SnapshotJson=ConvertTo-Json -InputObject $Owner.Snapshot -Compress
    } catch {$Owner.Ready=$false;$Owner.Error=$_.Exception.Message;throw}
}
