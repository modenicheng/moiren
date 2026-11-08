#requires -Version 5.1
# Read-only composition; this owner cannot authorize scheduler, ETW or audio.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'preflight-session.ps1')
. (Join-Path $PSScriptRoot 'session-identity.ps1')

function New-RemovalTargetReadiness {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$EvidenceDirectory,[switch]$UserRebootConfirmed,
        [ValidateRange(0,65535)][int]$WorkerSessionId=0,
        [string]$CollectorPath=(Join-Path $PSScriptRoot 'readonly-preflight.ps1'),[string]$CollectorSha256)
    $preflight=New-RemovalPreflightSession -EvidenceDirectory $EvidenceDirectory -UserRebootConfirmed:$UserRebootConfirmed `
        -CollectorPath $CollectorPath -CollectorSha256 $CollectorSha256
    $user=New-RemovalSessionIdentity -Sid 'S-1-5-21-3080296863-1242338503-1346362208-1001' -WorkerSessionId $WorkerSessionId
    $config=ConvertTo-Json -InputObject ([ordered]@{EvidenceDirectory=$EvidenceDirectory;RequestedWorkerSessionId=$WorkerSessionId}) -Compress
    [pscustomobject]@{EvidenceDirectory=$EvidenceDirectory;RequestedWorkerSessionId=$WorkerSessionId;ConfigJson=$config
        Preflight=$preflight;RetainedPreflight=$preflight;UserSession=$user;RetainedUserSession=$user
        StartAttempted=$false;Ready=$false;Error=$null;WorkerIdentityJson=$null
        ReceiptAttempted=$false;ReceiptPin=$null;ReceiptPinJson=$null;NativeApproved=$false;Qualified=$false}
}

function Get-RemovalTargetWorkerIdentity($Owner) {
    $s=$Owner.RetainedUserSession.Snapshot
    [ordered]@{Sid=$s.Sid;SessionId=$s.SessionId;ProcessId=$s.ProcessId;CreationUtcTicks=$s.CreationUtcTicks
        ExecutablePath=$s.ExecutablePath;IsAdministrator=$s.IsAdministrator;IsElevated=$s.IsElevated}
}

function Assert-RemovalTargetOwner($Owner) {
    Assert-RemovalTaskHost
    $config=ConvertTo-Json -InputObject ([ordered]@{EvidenceDirectory=$Owner.EvidenceDirectory;RequestedWorkerSessionId=$Owner.RequestedWorkerSessionId}) -Compress
    if($config -cne $Owner.ConfigJson -or -not[Object]::ReferenceEquals($Owner.Preflight,$Owner.RetainedPreflight) -or
        -not[Object]::ReferenceEquals($Owner.UserSession,$Owner.RetainedUserSession) -or
        $Owner.Preflight.EvidenceDirectory -cne $Owner.EvidenceDirectory -or
        $Owner.UserSession.RequestedSessionId -ne $Owner.RequestedWorkerSessionId -or
        $Owner.NativeApproved -isnot [bool] -or $Owner.NativeApproved -or
        $Owner.Qualified -isnot [bool] -or $Owner.Qualified){throw 'Original readonly target owner/configuration changed.'}
}

function Start-RemovalTargetReadiness {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($Owner.StartAttempted){throw 'Target readiness already attempted; retain original owners, never rediscover/relaunch.'}
    $Owner.StartAttempted=$true
    try {
        Assert-RemovalTargetOwner $Owner
        $null=Read-RemovalSessionIdentity $Owner.RetainedUserSession
        $Owner.WorkerIdentityJson=ConvertTo-Json -InputObject (Get-RemovalTargetWorkerIdentity $Owner) -Compress
        # Controller collection retains its real (possibly zero) SSH session.
        # The observed Limited user session is separate, never a relabelled DTO.
        Start-RemovalPreflightSession $Owner.RetainedPreflight
    } catch {$Owner.Ready=$false;$Owner.Error=$_.Exception.Message;throw}
}

function Save-RemovalTargetReadiness($Owner) {
    if($Owner.ReceiptAttempted){throw 'Target readiness receipt already attempted; no overwrite/adoption.'}
    $Owner.ReceiptAttempted=$true
    $p=$Owner.RetainedPreflight
    $receipt=[ordered]@{SchemaVersion=1;NativeApproved=$false;Qualified=$false;AudioAccessProven=$false
        ObservedUtc=[DateTimeOffset]::UtcNow.ToString('o');ExecutionMode=$p.ExecutionMode
        ControllerPid=$p.ControllerPid;ControllerCreationUtcTicks=$p.ControllerCreationUtcTicks;ControllerSession=$p.ControllerSession
        WorkerIdentity=(Get-RemovalTargetWorkerIdentity $Owner);WorkerObservedUtc=$Owner.RetainedUserSession.ObservedUtc
        PreflightReceipt=[ordered]@{Path=$p.ReceiptPin.Path;Bytes=$p.ReceiptPin.Bytes;Sha256=$p.ReceiptPin.Sha256}}
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $receipt -Depth 8))
    if($bytes.Length -gt 65536){throw 'Target readiness receipt exceeds 64 KiB cap.'}
    $path=Join-Path $Owner.EvidenceDirectory 'target-readiness.json';Assert-FiniteToolPath $path $false
    $file=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$file.Write($bytes,0,$bytes.Length);$file.Flush($true)}finally{$file.Dispose()}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$hash=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}finally{$sha.Dispose()}
    if((Get-Item -LiteralPath $path).Length -ne $bytes.Length -or (Get-FileHash -LiteralPath $path).Hash -cne $hash){throw 'Target readiness receipt readback differs.'}
    $Owner.ReceiptPin=[pscustomobject][ordered]@{Path=$path;Bytes=$bytes.Length;Sha256=$hash}
    $Owner.ReceiptPinJson=ConvertTo-Json -InputObject $Owner.ReceiptPin -Compress
}

function Assert-RemovalTargetReadiness {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    try {
        Assert-RemovalTargetOwner $Owner
        if(-not$Owner.StartAttempted -or -not$Owner.Ready -or $Owner.Error -or $null -eq $Owner.ReceiptPin){throw 'Target readiness missing or refused.'}
        Assert-RemovalPreflightSession $Owner.RetainedPreflight
        Assert-RemovalSessionIdentity $Owner.RetainedUserSession
        if((ConvertTo-Json -InputObject (Get-RemovalTargetWorkerIdentity $Owner) -Compress) -cne $Owner.WorkerIdentityJson){throw 'Original observed worker identity changed.'}
        $pin=$Owner.ReceiptPin
        if((ConvertTo-Json -InputObject $pin -Compress) -cne $Owner.ReceiptPinJson){throw 'Original target receipt pin changed.'}
        Assert-FiniteToolPath $pin.Path $false
        if($pin.Path -cne (Join-Path $Owner.EvidenceDirectory 'target-readiness.json') -or
            (Get-Item -LiteralPath $pin.Path).Length -ne $pin.Bytes -or (Get-FileHash -LiteralPath $pin.Path).Hash -cne $pin.Sha256){throw 'Original target readiness evidence changed.'}
    } catch {$Owner.Ready=$false;if(-not$Owner.Error){$Owner.Error=$_.Exception.Message};throw}
}

function Observe-RemovalTargetReadiness {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[ValidateRange(1,120000)][int]$ObservationMilliseconds=25)
    $ErrorActionPreference='Stop';$needs=$false;$p=$Owner.RetainedPreflight
    try {
        # Drain original inner work even if the public owner/session/evidence was
        # replaced or a previous observation failed. Failure never releases it.
        if($null -ne $p.RetainedOwner -and -not$p.RetainedOwner.Closed){
            $needs=$true
            $r=Observe-RemovalPreflightSession $p -ObservationMilliseconds $ObservationMilliseconds
            $needs=$r.NeedsObservation
        }
        Assert-RemovalTargetOwner $Owner
        if($Owner.Error){throw $Owner.Error}
        if(-not$Owner.StartAttempted){throw 'Target readiness has not been dispatched.'}
        if($p.Error){throw $p.Error}
        if($Owner.Ready){Assert-RemovalTargetReadiness $Owner}
        elseif($p.Ready){
            Assert-RemovalPreflightSession $p
            Assert-RemovalSessionIdentity $Owner.RetainedUserSession
            if((ConvertTo-Json -InputObject (Get-RemovalTargetWorkerIdentity $Owner) -Compress) -cne $Owner.WorkerIdentityJson){throw 'Original observed worker changed during collection.'}
            Save-RemovalTargetReadiness $Owner;$Owner.Ready=$true
            Assert-RemovalTargetReadiness $Owner
        }
    } catch {$Owner.Ready=$false;if(-not$Owner.Error){$Owner.Error=$_.Exception.Message}}
    [pscustomobject]@{Ready=$Owner.Ready;NeedsObservation=$needs;NativeApproved=$false;Qualified=$false
        ReceiptPin=$Owner.ReceiptPin;Error=$Owner.Error}
}
