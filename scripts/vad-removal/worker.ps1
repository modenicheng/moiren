#requires -Version 5.1
param([Parameter(Mandatory=$true)][string]$TaskPath,
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{32}$')][string]$Nonce,
    [Parameter(Mandatory=$true)][ValidateRange(1,65535)][int]$ExpectedSessionId,
    [Parameter(Mandatory=$true)][string]$RunDirectory,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$WorkerSha256,
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$RunSpecSha256)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'

# The manifest cannot supply its own validator: reject redirected ancestors
# before reading configuration or importing any staged library definitions.
function Assert-RemovalWorkerSourcePath([string]$Path) {
    if($Path -notmatch '\A[A-Za-z]:\\' -or $Path -match '[\x00-\x1f"*?]' -or
        $Path.Substring(2).Contains(':') -or [IO.Path]::GetFullPath($Path) -cne $Path -or $Path.EndsWith('\')){
        throw 'Canonical worker source path required.'
    }
    $cursor=$Path
    while($cursor){
        $item=Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
        if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Reparse worker source path refused.'}
        if($cursor -cne $Path -and -not $item.PSIsContainer){throw 'Non-directory worker source ancestor.'}
        $cursor=[IO.Path]::GetDirectoryName($cursor)
    }
}

function Read-RemovalWorkerPinnedBytes {
    param([string]$Path,[string]$ExpectedSha256,[int]$MinimumBytes,[int]$MaximumBytes,
        [string]$Label,[string]$MismatchMessage,[Collections.Generic.List[object]]$Locks)
    Assert-RemovalWorkerSourcePath $Path
    $file=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $Locks.Add($file)
    Assert-RemovalWorkerSourcePath $Path
    # Cap before hashing; decode/parse these exact bytes, never reopen the path.
    if($file.Length -lt $MinimumBytes -or $file.Length -gt $MaximumBytes){throw ($Label+' length invalid.')}
    $bytes=New-Object byte[] ([int]$file.Length);$offset=0
    while($offset -lt $bytes.Length){
        $count=$file.Read($bytes,$offset,$bytes.Length-$offset)
        if($count -eq 0){throw ($Label+' read incomplete.')};$offset+=$count
    }
    if($file.ReadByte() -ne -1){throw ($Label+' grew during read.')}
    $sha=[Security.Cryptography.SHA256]::Create()
    try {$hash=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}
    finally {$sha.Dispose()}
    if($hash -ine $ExpectedSha256){throw $MismatchMessage}
    return ,$bytes
}

$sourceLocks=New-Object Collections.Generic.List[object]
$script:RemovalWorkerProbeOwner=$null
try {
    Assert-RemovalWorkerSourcePath $RunDirectory
    if([IO.Path]::GetDirectoryName($PSCommandPath) -cne $RunDirectory){throw 'Worker entry/source path differs.'}
    $null=Read-RemovalWorkerPinnedBytes -Path $PSCommandPath -ExpectedSha256 $WorkerSha256 `
        -MinimumBytes 1 -MaximumBytes 262144 -Label 'Worker entry' -MismatchMessage 'Worker entry/source path differs.' -Locks $sourceLocks
    $path=Join-Path $RunDirectory 'run-spec.json'
    $bytes=Read-RemovalWorkerPinnedBytes -Path $path -ExpectedSha256 $RunSpecSha256 `
        -MinimumBytes 2 -MaximumBytes 8192 -Label 'Run configuration' -MismatchMessage 'Pinned run configuration differs.' -Locks $sourceLocks
    try {$text=[Text.UTF8Encoding]::new($false,$true).GetString($bytes)}
    catch {throw 'Run configuration UTF-8 invalid.'}
    try {$run=ConvertFrom-Json -InputObject $text -ErrorAction Stop}
    catch {throw 'Run configuration JSON invalid.'}
    if($run.SchemaVersion -ne 1 -or $run.Nonce -cne $Nonce -or $run.RunDirectory -cne $RunDirectory -or
        $run.SessionId -ne $ExpectedSessionId -or $run.Libraries -isnot [Array] -or $run.Libraries.Count -ne 4){throw 'Wrong exact run configuration.'}
    $names=@('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1')
    for($i=0;$i -lt $names.Count;$i++){
        $pin=$run.Libraries[$i]
        if($pin.Name -cne $names[$i] -or $pin.Sha256 -cnotmatch '^[A-F0-9]{64}$'){throw 'Wrong ordered source manifest.'}
        $source=Join-Path $RunDirectory $pin.Name
        $null=Read-RemovalWorkerPinnedBytes -Path $source -ExpectedSha256 $pin.Sha256 `
            -MinimumBytes 1 -MaximumBytes 262144 -Label 'Worker library' -MismatchMessage 'Worker library pin differs.' -Locks $sourceLocks
    }
    # Each path-based nested dot-source may reopen for reading, but the retained
    # FileShare.Read handles deny writing, deleting or replacing pinned files.
    foreach($name in ($names+@('worker.ps1','run-spec.json'))){Assert-RemovalWorkerSourcePath (Join-Path $RunDirectory $name)}
    . (Join-Path $RunDirectory 'worker-boundary.ps1')
    Assert-RemovalTaskPath $RunDirectory
    $current=[Diagnostics.Process]::GetCurrentProcess()
    $spec=New-RemovalProbeSpec -Executable $run.Executable -ExpectedSha256 $run.ExpectedSha256 `
        -RunDirectory $RunDirectory -Nonce $Nonce -SessionId $ExpectedSessionId -Sid $run.Sid `
        -ControllerPid $run.ControllerPid -ControllerCreationUtcTicks $run.ControllerCreationUtcTicks `
        -WorkerPid $current.Id -WorkerCreationUtcTicks $current.StartTime.ToUniversalTime().Ticks -ProbeArguments $run.ProbeArguments
    $result=Invoke-RemovalWorker -Spec $spec -TaskPath $TaskPath -WorkerPath $PSCommandPath -WorkerSha256 $WorkerSha256
    exit $result.WorkerExitCode
} catch {
    [Console]::Error.WriteLine('Removal worker failed/unknown: '+$_.Exception.Message)
    # An unexpected exception is not permission to exit a process that owns a
    # dispatched child or its pipes. Keep observing this exact owner, including
    # ambiguous starts and transient observation/close failures; never relaunch.
    $retainedOwner=$script:RemovalWorkerProbeOwner
    $observationErrorReported=$false
    while($null -ne $retainedOwner -and $retainedOwner.LaunchAttempted -and -not $retainedOwner.Closed){
        try {
            $observation=Observe-RemovalWorkerProbe $retainedOwner -ObservationMilliseconds 100
            if(-not $observation.NeedsObservation){
                Close-RemovalWorkerProbeOwner $retainedOwner
                break
            }
        } catch {
            if(-not $observationErrorReported){
                [Console]::Error.WriteLine('Original removal probe observation failed; retaining same owner.')
                $observationErrorReported=$true
            }
        }
        Start-Sleep -Milliseconds 100
    }
    exit 1
} finally {
    foreach($file in $sourceLocks){$file.Dispose()}
}
