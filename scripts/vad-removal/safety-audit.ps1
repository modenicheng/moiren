#requires -Version 5.1
# Read-only after collection owns the same runspace/children until natural
# completion. Safety evidence never authorizes audio, mutation or qualification.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'target-readiness.ps1')

function ConvertTo-RemovalSafetyState($Snapshot) {
    $p=$Snapshot;$d=$p.Driver;$s=$p.Security
    # Explicit key selectors sort runspace dictionaries and JSON objects alike.
    [ordered]@{
        host=$p.host;sid=$p.sid;session=[int]$p.session
        # PS7 may decode archived JSON dates as DateTime; live schema validation
        # already requires explicit-zone text before this canonical projection.
        BootUtc=([DateTimeOffset]$p.BootUtc).ToUniversalTime().ToString('o')
        Driver=[ordered]@{Version=$d.Version;Inf=$d.Inf;State=$d.State;InstanceId=$d.InstanceId
            PnpStatus=$d.PnpStatus;ProblemCode=[long]$d.ProblemCode;SysPath=$d.SysPath
            SysSha256=$d.SysSha256;InfSha256=$d.InfSha256}
        ExternalOwners=@($p.ExternalOwners|Sort-Object {[long]$_.Pid}|ForEach-Object {
            [ordered]@{Pid=[long]$_.Pid;Name=$_.Name;Session=[int]$_.Session
                CreationUnixMs=[long]$_.CreationUnixMs;ExecutablePath=$_.ExecutablePath}
        })
        Defaults=@($p.Defaults|Sort-Object {$_.flow},{$_.role}|ForEach-Object {[ordered]@{flow=$_.flow;role=$_.role;id=$_.id}})
        Render=@($p.Render|Sort-Object {$_.id}|ForEach-Object {[ordered]@{id=$_.id;volume_scalar=[double]$_.volume_scalar;muted=$_.muted}})
        Security=[ordered]@{SecureBoot=$s.SecureBoot;Vbs=[int]$s.Vbs;CodeIntegrity=[int]$s.CodeIntegrity
            UserCodeIntegrity=[int]$s.UserCodeIntegrity;Configured=@($s.Configured|Sort-Object)
            Running=@($s.Running|Sort-Object);Bcd=$s.Bcd.ToUpperInvariant()}
    }
}

function New-RemovalSafetyAudit {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)]$BeforeReadiness,
        [Parameter(Mandatory=$true)][string]$EvidenceDirectory,[switch]$UserRebootConfirmed,
        [string]$CollectorPath=(Join-Path $PSScriptRoot 'readonly-preflight.ps1'),[string]$CollectorSha256)
    if(-not$UserRebootConfirmed){throw 'User reboot confirmation required before after-safety collection.'}
    Assert-RemovalTaskHost
    $before=$BeforeReadiness.RetainedPreflight
    # Construction can occur after a long/failed diagnostic. Require the actual
    # completed, previously published baseline, not renewed readiness freshness,
    # Explorer liveness or a detached caller DTO. Revalidate its evidence after
    # collecting, so a changed source/receipt cannot suppress after observation.
    if(-not$BeforeReadiness.StartAttempted -or -not$BeforeReadiness.ReceiptAttempted -or
        $BeforeReadiness.ReceiptPinJson -isnot [string] -or -not$BeforeReadiness.ReceiptPinJson -or
        $BeforeReadiness.WorkerIdentityJson -isnot [string] -or -not$BeforeReadiness.WorkerIdentityJson -or
        -not$before.StartAttempted -or -not$before.ObservationComplete -or -not$before.ReceiptAttempted -or
        $before.SnapshotJson -isnot [string] -or -not$before.SnapshotJson -or
        $before.ReceiptPinJson -isnot [string] -or -not$before.ReceiptPinJson -or
        $null -eq $before.RetainedOwner -or -not$before.RetainedOwner.Completed -or -not$before.RetainedOwner.Closed){
        throw 'Original completed and evidenced target readiness required; detached or unfinished before state refused.'
    }
    $after=New-RemovalPreflightSession -EvidenceDirectory $EvidenceDirectory -UserRebootConfirmed `
        -CollectorPath $CollectorPath -CollectorSha256 $CollectorSha256 -ObservationMode Audit `
        -ReadToolPins (ConvertFrom-Json -InputObject $before.ReadToolPinsJson)
    $binding=ConvertTo-Json -InputObject ([ordered]@{EvidenceDirectory=$EvidenceDirectory
        BeforeSnapshotJson=$before.SnapshotJson;BeforePreflightPinJson=$before.ReceiptPinJson
        BeforeReadinessPinJson=$BeforeReadiness.ReceiptPinJson;BeforeWorkerIdentityJson=$BeforeReadiness.WorkerIdentityJson
        AfterSourceIdentityJson=$after.SourceIdentityJson;ReadToolPinsJson=$before.ReadToolPinsJson}) -Compress
    [pscustomobject]@{BeforeReadiness=$BeforeReadiness;RetainedBeforeReadiness=$BeforeReadiness;RetainedBeforePreflight=$before
        AfterPreflight=$after;RetainedAfterPreflight=$after;EvidenceDirectory=$EvidenceDirectory;BindingJson=$binding
        BeforeSnapshotJson=$before.SnapshotJson;BeforePreflightPinJson=$before.ReceiptPinJson
        BeforeReadinessPinJson=$BeforeReadiness.ReceiptPinJson;BeforeWorkerIdentityJson=$BeforeReadiness.WorkerIdentityJson
        AfterSourceIdentityJson=$after.SourceIdentityJson;ReadToolPinsJson=$before.ReadToolPinsJson;StartAttempted=$false;StartedUtc=$null
        Complete=$false;Safe=$false;Unsafe=$false;Differences=@();ExternalIdentityChanges=@()
        Error=$null;NativeApproved=$false;Qualified=$false
        ReceiptAttempted=$false;ReceiptPin=$null;ReceiptPinJson=$null;AfterSnapshotJson=$null;ResultJson=$null}
}

function Assert-RemovalSafetyOwner($Owner) {
    Assert-RemovalTaskHost
    if(-not[Object]::ReferenceEquals($Owner.BeforeReadiness,$Owner.RetainedBeforeReadiness) -or
        -not[Object]::ReferenceEquals($Owner.AfterPreflight,$Owner.RetainedAfterPreflight) -or
        $Owner.RetainedAfterPreflight.ObservationMode -cne 'Audit' -or
        $Owner.RetainedAfterPreflight.ReadToolPinsJson -cne $Owner.ReadToolPinsJson -or
        $Owner.RetainedAfterPreflight.EvidenceDirectory -cne $Owner.EvidenceDirectory -or
        $Owner.NativeApproved -isnot [bool] -or $Owner.NativeApproved -or
        $Owner.Qualified -isnot [bool] -or $Owner.Qualified){throw 'Original safety audit owner/configuration differs.'}
    $binding=ConvertTo-Json -InputObject ([ordered]@{EvidenceDirectory=$Owner.EvidenceDirectory
        BeforeSnapshotJson=$Owner.BeforeSnapshotJson;BeforePreflightPinJson=$Owner.BeforePreflightPinJson
        BeforeReadinessPinJson=$Owner.BeforeReadinessPinJson;BeforeWorkerIdentityJson=$Owner.BeforeWorkerIdentityJson
        AfterSourceIdentityJson=$Owner.RetainedAfterPreflight.SourceIdentityJson;ReadToolPinsJson=$Owner.ReadToolPinsJson}) -Compress
    if($binding -cne $Owner.BindingJson -or
        $Owner.AfterSourceIdentityJson -cne $Owner.RetainedAfterPreflight.SourceIdentityJson){throw 'Original safety audit bindings changed; no repinning.'}
}

function Assert-RemovalSafetyBaseline($Owner) {
    $before=$Owner.RetainedBeforeReadiness;$p=$Owner.RetainedBeforePreflight
    Assert-RemovalTargetOwner $before
    if(-not[Object]::ReferenceEquals($p,$before.RetainedPreflight) -or
        $p.ObservationMode -cne 'Admission' -or $p.ReadToolPinsJson -cne $Owner.ReadToolPinsJson -or
        $p.SnapshotJson -cne $Owner.BeforeSnapshotJson -or
        $p.ReceiptPinJson -cne $Owner.BeforePreflightPinJson -or
        $before.ReceiptPinJson -cne $Owner.BeforeReadinessPinJson -or
        $before.WorkerIdentityJson -cne $Owner.BeforeWorkerIdentityJson -or
        (ConvertTo-Json -InputObject (Get-RemovalTargetWorkerIdentity $before) -Compress) -cne $Owner.BeforeWorkerIdentityJson){
        throw 'Original accepted before-readiness evidence differs.'
    }
    # Evidence validation checks immutable snapshots/pins without rediscovering
    # Explorer, reopening an external PID or renewing expired admission.
    Assert-RemovalPreflightEvidence $p
    $pin=$before.ReceiptPin
    if((ConvertTo-Json -InputObject $pin -Compress) -cne $Owner.BeforeReadinessPinJson -or
        $pin.Path -cne (Join-Path $before.EvidenceDirectory 'target-readiness.json')){throw 'Original before-readiness receipt pin differs.'}
    Assert-FiniteToolPath $pin.Path $false
    if((Get-Item -LiteralPath $pin.Path).Length -ne $pin.Bytes -or (Get-FileHash -LiteralPath $pin.Path).Hash -cne $pin.Sha256){
        throw 'Original before-readiness receipt changed.'
    }
}

function Start-RemovalSafetyAudit {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($Owner.StartAttempted){throw 'Safety audit already attempted; observe original collection, never restart.'}
    $Owner.StartAttempted=$true
    try {
        Assert-RemovalSafetyOwner $Owner
        $Owner.StartedUtc=[DateTimeOffset]::UtcNow.ToString('o')
        # Before evidence may have become invalid after construction. That is a
        # safety refusal to record, not a reason to skip real after observation.
        Start-RemovalPreflightSession $Owner.RetainedAfterPreflight
    } catch {$Owner.Error=$_.Exception.Message;$Owner.Unsafe=$true;throw}
}

function Get-RemovalSafetyComparison($Owner) {
    $differences=New-Object Collections.Generic.List[string]
    $externalChanges=New-Object Collections.Generic.List[object]
    $errors=New-Object Collections.Generic.List[string]
    if($Owner.Error){$errors.Add($Owner.Error)}
    try {Assert-RemovalSafetyBaseline $Owner}
    catch {$differences.Add('BeforeEvidence');$errors.Add($_.Exception.Message)}
    $after=$Owner.RetainedAfterPreflight
    $valid=$false
    try {
        Assert-RemovalPreflightObservation $after
        $snapshot=$after.RetainedOwner.Shared.Snapshot
        if((Convert-RemovalPreflightTime $snapshot.CollectorStartedUtc) -lt (Convert-RemovalPreflightTime $Owner.StartedUtc)){
            throw 'After snapshot predates this original audit dispatch.'
        }
        $valid=$true
    } catch {$differences.Add('AfterObservation');$errors.Add($_.Exception.Message);if($after.Error){$errors.Add($after.Error)}}
    if($valid){
        $before=ConvertTo-RemovalSafetyState (ConvertFrom-Json -InputObject $Owner.BeforeSnapshotJson)
        $current=ConvertTo-RemovalSafetyState $snapshot
        foreach($field in $before.Keys){
            $left=ConvertTo-Json -InputObject $before[$field] -Depth 8 -Compress
            $right=ConvertTo-Json -InputObject $current[$field] -Depth 8 -Compress
            if($left -cne $right){$differences.Add($field)}
        }
        foreach($original in $before.ExternalOwners){
            $matches=@($current.ExternalOwners|Where-Object{$_.Pid -eq $original.Pid})
            if($matches.Count -eq 0){
                $externalChanges.Add([ordered]@{Kind='Missing';Pid=$original.Pid;Before=$original;After=$null})
            } elseif((ConvertTo-Json -InputObject $original -Compress) -cne (ConvertTo-Json -InputObject $matches[0] -Compress)){
                $externalChanges.Add([ordered]@{Kind='Replaced';Pid=$original.Pid;Before=$original;After=$matches[0]})
            }
        }
        foreach($observed in $current.ExternalOwners){
            if(@($before.ExternalOwners|Where-Object{$_.Pid -eq $observed.Pid}).Count -eq 0){
                $externalChanges.Add([ordered]@{Kind='Added';Pid=$observed.Pid;Before=$null;After=$observed})
            }
        }
        foreach($field in @('NativeOwners','RunningTasks','PresentRawChildren')){
            if($snapshot[$field].Count -ne 0){$differences.Add($field)}
        }
    }
    if($differences.Count -and -not$errors.Count){$errors.Add('Safety state differs: '+($differences.ToArray() -join ', '))}
    [pscustomobject]@{Safe=($differences.Count -eq 0 -and $errors.Count -eq 0)
        Differences=$differences.ToArray();ExternalIdentityChanges=$externalChanges.ToArray()
        Error=if($errors.Count){$errors.ToArray() -join '; '}else{$null}}
}

function Get-RemovalSafetyResultJson($Owner) {
    ConvertTo-Json -InputObject ([ordered]@{Safe=$Owner.Safe;Unsafe=$Owner.Unsafe
        Differences=$Owner.Differences;ExternalIdentityChanges=$Owner.ExternalIdentityChanges;Error=$Owner.Error}) -Depth 8 -Compress
}

function Save-RemovalSafetyReceipt($Owner,$Comparison) {
    if($Owner.ReceiptAttempted){throw 'Safety receipt already attempted; no overwrite/retry.'}
    $Owner.ReceiptAttempted=$true
    $p=$Owner.RetainedAfterPreflight
    $snapshot=if($null -ne $p.RetainedOwner){$p.RetainedOwner.Shared.Snapshot}else{$null}
    $Owner.AfterSnapshotJson=ConvertTo-Json -InputObject $snapshot -Depth 16 -Compress
    $Owner.Safe=$Comparison.Safe;$Owner.Unsafe=-not$Comparison.Safe
    $Owner.Differences=$Comparison.Differences;$Owner.ExternalIdentityChanges=$Comparison.ExternalIdentityChanges
    $Owner.Error=$Comparison.Error
    $Owner.ResultJson=Get-RemovalSafetyResultJson $Owner
    $receipt=[ordered]@{SchemaVersion=1;Safe=$Owner.Safe;Unsafe=$Owner.Unsafe;NativeApproved=$false;Qualified=$false
        StartedUtc=$Owner.StartedUtc;ObservedUtc=[DateTimeOffset]::UtcNow.ToString('o')
        Differences=$Owner.Differences;ExternalIdentityChanges=$Owner.ExternalIdentityChanges;Error=$Owner.Error
        BeforeReadinessReceipt=(ConvertFrom-Json -InputObject $Owner.BeforeReadinessPinJson)
        BeforeReadinessRefusal=$Owner.RetainedBeforeReadiness.Error
        BeforePreflightReceipt=(ConvertFrom-Json -InputObject $Owner.BeforePreflightPinJson)
        BeforeSnapshot=(ConvertFrom-Json -InputObject $Owner.BeforeSnapshotJson)
        AfterPreflightReceipt=$p.ReceiptPin;AfterSourceIdentity=(ConvertFrom-Json -InputObject $Owner.AfterSourceIdentityJson)
        AfterSnapshot=$snapshot}
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $receipt -Depth 20))
    if($bytes.Length -gt 2097152){throw 'Safety audit receipt exceeds 2 MiB cap.'}
    $null=[IO.Directory]::CreateDirectory($Owner.EvidenceDirectory)
    $path=Join-Path $Owner.EvidenceDirectory 'safety-audit.json';Assert-FiniteToolPath $path $false
    $file=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$file.Write($bytes,0,$bytes.Length);$file.Flush($true)}finally{$file.Dispose()}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$hash=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}finally{$sha.Dispose()}
    if((Get-Item -LiteralPath $path).Length -ne $bytes.Length -or (Get-FileHash -LiteralPath $path).Hash -cne $hash){
        throw 'Safety audit receipt readback differs.'
    }
    $Owner.ReceiptPin=[pscustomobject][ordered]@{Path=$path;Bytes=$bytes.Length;Sha256=$hash}
    $Owner.ReceiptPinJson=ConvertTo-Json -InputObject $Owner.ReceiptPin -Compress
    $Owner.Complete=$true
}

function Assert-RemovalSafetyReceipt($Owner) {
    Assert-RemovalSafetyOwner $Owner
    $p=$Owner.RetainedAfterPreflight
    if(-not$Owner.Complete -or $null -eq $Owner.ReceiptPin -or
        ($null -ne $p.RetainedOwner -and -not$p.RetainedOwner.Closed) -or
        (Get-RemovalSafetyResultJson $Owner) -cne $Owner.ResultJson){throw 'Original complete safety result differs or is missing.'}
    $snapshot=if($null -ne $p.RetainedOwner){$p.RetainedOwner.Shared.Snapshot}else{$null}
    if((ConvertTo-Json -InputObject $snapshot -Depth 16 -Compress) -cne $Owner.AfterSnapshotJson){throw 'Original after snapshot changed.'}
    $pin=$Owner.ReceiptPin
    if((ConvertTo-Json -InputObject $pin -Compress) -cne $Owner.ReceiptPinJson -or
        $pin.Path -cne (Join-Path $Owner.EvidenceDirectory 'safety-audit.json')){throw 'Original safety receipt pin changed.'}
    Assert-FiniteToolPath $pin.Path $false
    if((Get-Item -LiteralPath $pin.Path).Length -ne $pin.Bytes -or (Get-FileHash -LiteralPath $pin.Path).Hash -cne $pin.Sha256){
        throw 'Original safety receipt changed.'
    }
}

function Assert-RemovalSafetyAudit {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    try {
        Assert-RemovalSafetyReceipt $Owner
        if(-not$Owner.Safe -or $Owner.Unsafe -or $Owner.Error){throw 'After-safety audit is unsafe or unknown.'}
        $comparison=Get-RemovalSafetyComparison $Owner
        if(-not$comparison.Safe){throw ('Original safety evidence refused: '+$comparison.Error)}
    } catch {$Owner.Safe=$false;$Owner.Unsafe=$true;if(-not$Owner.Error){$Owner.Error=$_.Exception.Message};throw}
}

function Observe-RemovalSafetyAudit {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[ValidateRange(1,120000)][int]$ObservationMilliseconds=25)
    $ErrorActionPreference='Stop';$needs=$false;$p=$Owner.RetainedAfterPreflight
    try {
        # Settle original work first, even if baseline, public owners or pins
        # changed. Never cancel/dispose an unfinished runspace or child handle.
        if($null -ne $p.RetainedOwner -and -not$p.RetainedOwner.Closed){
            $needs=$true
            $r=Observe-RemovalPreflightSession $p -ObservationMilliseconds $ObservationMilliseconds
            $needs=$r.NeedsObservation
        }
        if(-not$Owner.StartAttempted){throw 'Safety audit has not been dispatched.'}
        if(-not$needs){
            Assert-RemovalSafetyOwner $Owner
            if($Owner.Complete){
                Assert-RemovalSafetyReceipt $Owner
                if($Owner.Safe){Assert-RemovalSafetyAudit $Owner}
            } elseif(-not$Owner.ReceiptAttempted){
                $comparison=Get-RemovalSafetyComparison $Owner
                Save-RemovalSafetyReceipt $Owner $comparison
                Assert-RemovalSafetyReceipt $Owner
            }
        }
    } catch {$Owner.Safe=$false;$Owner.Unsafe=$true;if(-not$Owner.Error){$Owner.Error=$_.Exception.Message}}
    [pscustomobject]@{Complete=$Owner.Complete;NeedsObservation=$needs;Safe=$Owner.Safe;Unsafe=$Owner.Unsafe
        Differences=$Owner.Differences;ExternalIdentityChanges=$Owner.ExternalIdentityChanges
        ReceiptPin=$Owner.ReceiptPin;Error=$Owner.Error;NativeApproved=$false;Qualified=$false}
}
