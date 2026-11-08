#requires -Version 5.1
# Retain the real collector invocation. Readiness is not native authorization.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
. (Join-Path $PSScriptRoot 'task-boundary.ps1')
. (Join-Path $PSScriptRoot 'collector-worker.ps1')

function Assert-RemovalPreflightInteger($Value,[long]$Minimum,[long]$Maximum) {
    if(($Value -isnot [int] -and $Value -isnot [long] -and $Value -isnot [uint32] -and $Value -isnot [uint64]) -or
        $Value -lt $Minimum -or $Value -gt $Maximum){throw 'Missing/out-of-range typed integer in preflight.'}
}

function Convert-RemovalPreflightTime($Value) {
    if($Value -isnot [string] -or $Value -notmatch '(?:Z|[+-]\d\d:\d\d)$'){throw 'Explicit-zone preflight timestamp required.'}
    return [DateTimeOffset]::Parse($Value,[Globalization.CultureInfo]::InvariantCulture)
}

function Assert-RemovalPreflightSnapshot($Snapshot,[DateTimeOffset]$Now,[int]$ControllerSession) {
    $p=$Snapshot
    if($p -isnot [Collections.IDictionary] -or $p.schema_version -ne 1 -or
        $p.host -cne 'LAPTOP-NP49RG6H' -or $p.sid -cne 'S-1-5-21-3080296863-1242338503-1346362208-1001'){
        throw 'Wrong/missing preflight target identity.'
    }
    Assert-RemovalPreflightInteger $p.session 0 65535
    if($p.session -ne $ControllerSession){throw 'Collector session differs from actual controller; no desktop-session relabelling.'}
    foreach($field in @('BootAfterDeployment','SignaturesVerified','CatalogMembersVerified')){
        if($p[$field] -isnot [bool] -or -not$p[$field]){throw ('Unverified preflight '+$field)}
    }
    foreach($field in @('NativeTestStarted','RebootRequested','Qualified')){
        if($p[$field] -isnot [bool] -or $p[$field]){throw ('Unexpected collector activity '+$field)}
    }
    $boot=Convert-RemovalPreflightTime $p.BootUtc;$observed=Convert-RemovalPreflightTime $p.ObservedUtc
    $started=Convert-RemovalPreflightTime $p.CollectorStartedUtc
    $processStarted=Convert-RemovalPreflightTime $p.ExternalOwnersObservationStartedUtc
    Assert-RemovalPreflightInteger $p.UptimeMilliseconds 0 315360000000
    $age=($Now-$observed).TotalSeconds
    if($boot -le [DateTimeOffset]'2026-11-07T16:10:12.358563Z' -or $boot -gt $started -or
        $started -gt $observed -or ($observed-$started).TotalSeconds -gt 120 -or $age -lt 0 -or $age -gt 120 -or
        $processStarted -lt $started -or $processStarted -gt $observed -or
        [Math]::Abs(($observed-$boot).TotalMilliseconds-[double]$p.UptimeMilliseconds) -gt 5000){
        throw 'No fresh corroborated post-install boot/collection interval.'
    }
    foreach($field in @('NativeOwners','RunningTasks','PresentRawChildren')){
        if($p[$field] -isnot [Array] -or $p[$field].Count -ne 0){throw ('Busy/unknown '+$field)}
    }
    $driver=$p.Driver
    if($null -eq $driver -or $driver.Version -cne '3.1.1.613' -or $driver.Inf -cne 'oem207.inf' -or
        $driver.State -cne 'Running' -or $driver.InstanceId -cne 'ROOT\MEDIA\0001' -or $driver.PnpStatus -cne 'OK' -or
        $driver.SysSha256 -cne '0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76' -or
        $driver.InfSha256 -cne '24574ED3BAE942EF7ED91601513F8F0B181272BF1F15D2067F0F6EE2014F827C'){
        throw 'Wrong/unhealthy frozen613 service-file/PnP provenance.'
    }
    Assert-RemovalPreflightInteger $driver.ProblemCode 0 0
    Assert-FiniteToolPath $driver.SysPath $false
    if($p.ExternalOwners -isnot [Array] -or $p.ExternalOwnersProvenance -cne 'ObservedComplete' -or
        $p.ExternalOwnersObservedUtc -cne $p.ObservedUtc){throw 'Fresh complete explicit external-owner observation required.'}
    $seen=@{}
    foreach($owner in $p.ExternalOwners){
        Assert-RemovalPreflightInteger $owner.Pid 1 4294967295
        Assert-RemovalPreflightInteger $owner.Session 0 65535
        Assert-RemovalPreflightInteger $owner.CreationUnixMs 1 253402300799999
        if($owner.Name -isnot [string] -or $owner.Name -cnotmatch '\A[A-Za-z0-9_.-]+\.exe\z' -or
            $seen.ContainsKey([string]$owner.Pid)){throw 'Malformed/duplicate external process identity.'}
        Assert-FiniteToolPath $owner.ExecutablePath $false;$seen[[string]$owner.Pid]=$true
    }
    if($p.Defaults -isnot [Array] -or $p.Defaults.Count -ne 6){throw 'All six exact default roles required.'}
    foreach($flow in @('capture','render')){foreach($role in @('communications','console','multimedia')){
        $rows=@($p.Defaults|Where-Object{$_.flow -ceq $flow -and $_.role -ceq $role})
        if($rows.Count -ne 1 -or $rows[0].id -isnot [string] -or -not$rows[0].id){throw 'Missing/duplicate exact default role.'}
    }}
    if($p.Render -isnot [Array] -or $p.Render.Count -lt 1){throw 'Explicit render controls required.'}
    $seen=@{}
    foreach($render in $p.Render){
        $v=$render.volume_scalar
        if($render.id -isnot [string] -or -not$render.id -or $seen.ContainsKey($render.id) -or
            ($v -isnot [double] -and $v -isnot [single] -and $v -isnot [decimal] -and $v -isnot [int] -and $v -isnot [long]) -or
            [double]::IsNaN([double]$v) -or $v -lt 0 -or $v -gt 1 -or $render.muted -isnot [bool]){throw 'Malformed render volume/mute identity.'}
        $seen[$render.id]=$true
    }
    $security=$p.Security
    if($null -eq $security -or $security.SecureBoot -isnot [bool] -or $security.Bcd -cnotmatch '\A[A-Fa-f0-9]{64}\z'){
        throw 'Incomplete security baseline.'
    }
    foreach($field in @('Vbs','CodeIntegrity','UserCodeIntegrity')){Assert-RemovalPreflightInteger $security.$field 0 2}
    foreach($field in @('Configured','Running')){
        if($security.$field -isnot [Array] -or $security.$field.Count -lt 1){throw 'Missing security service observations.'}
        foreach($value in $security.$field){Assert-RemovalPreflightInteger $value 0 65535}
    }
}

function New-RemovalPreflightSession {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$EvidenceDirectory,[switch]$UserRebootConfirmed,
        [string]$CollectorPath=(Join-Path $PSScriptRoot 'readonly-preflight.ps1'),[string]$CollectorSha256)
    if(-not$UserRebootConfirmed){throw 'User reboot confirmation required before target collection; never reboot automatically.'}
    Assert-RemovalTaskHost
    Assert-FiniteToolPath $EvidenceDirectory $false
    if(Test-Path -LiteralPath $EvidenceDirectory){throw 'Create-new preflight evidence directory required.'}
    $finite=Join-Path $PSScriptRoot 'finite-tool.ps1'
    $finiteHash='C3621DC9C881519006E01619D2C65E053C3A1F0FBFC100C0E247E3064D16651B'
    Assert-CollectorInput $finite $finiteHash
    if(-not$CollectorSha256){$CollectorSha256=(Get-FileHash -LiteralPath $CollectorPath -Algorithm SHA256).Hash}
    Assert-CollectorInput $CollectorPath $CollectorSha256
    $self=[Diagnostics.Process]::GetCurrentProcess()
    $mode=if($CollectorPath -ceq (Join-Path $PSScriptRoot 'readonly-preflight.ps1')){'RepositoryCollector'}else{'InjectedCollectorSource'}
    $sourceIdentity=ConvertTo-Json -InputObject ([ordered]@{FiniteToolPath=$finite;FiniteToolSha256=$finiteHash
        CollectorPath=$CollectorPath;CollectorSha256=$CollectorSha256;ExecutionMode=$mode}) -Compress
    [pscustomobject]@{ControllerPid=$self.Id;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks;ControllerSession=$self.SessionId
        EvidenceDirectory=$EvidenceDirectory;FiniteToolPath=$finite;FiniteToolSha256=$finiteHash
        CollectorPath=$CollectorPath;CollectorSha256=$CollectorSha256;CollectorOwner=$null;RetainedOwner=$null
        StartAttempted=$false;Ready=$false;ObservationComplete=$false;ReceiptAttempted=$false;ReceiptPin=$null;ReceiptPinJson=$null;SnapshotJson=$null
        ExecutionMode=$mode;SourceIdentityJson=$sourceIdentity
        Error=$null}
}

function Assert-RemovalPreflightSessionIdentity($Session) {
    $self=[Diagnostics.Process]::GetCurrentProcess()
    if($self.Id -ne $Session.ControllerPid -or $self.StartTime.ToUniversalTime().Ticks -ne $Session.ControllerCreationUtcTicks -or
        $self.SessionId -ne $Session.ControllerSession -or
        -not[Object]::ReferenceEquals($Session.CollectorOwner,$Session.RetainedOwner)){
        throw 'Original preflight controller/collector identity differs.'
    }
    $source=ConvertTo-Json -InputObject ([ordered]@{FiniteToolPath=$Session.FiniteToolPath;FiniteToolSha256=$Session.FiniteToolSha256
        CollectorPath=$Session.CollectorPath;CollectorSha256=$Session.CollectorSha256;ExecutionMode=$Session.ExecutionMode}) -Compress
    if($source -cne $Session.SourceIdentityJson){throw 'Original collector/helper source identity changed; no repinning.'}
    Assert-CollectorInput $Session.FiniteToolPath $Session.FiniteToolSha256
    Assert-CollectorInput $Session.CollectorPath $Session.CollectorSha256
}

function Start-RemovalPreflightSession {
    param([Parameter(Mandatory=$true,Position=0)]$Session)
    Assert-RemovalTaskHost;Assert-RemovalPreflightSessionIdentity $Session
    if($Session.StartAttempted){throw 'Collector already attempted; observe retained invocation, never restart.'}
    $Session.StartAttempted=$true
    $owner=Start-RemovalCollectorWorker -FiniteToolPath $Session.FiniteToolPath -FiniteToolSha256 $Session.FiniteToolSha256 `
        -CollectorPath $Session.CollectorPath -CollectorSha256 $Session.CollectorSha256 -EvidenceDirectory $Session.EvidenceDirectory
    $Session.RetainedOwner=$owner;$Session.CollectorOwner=$owner
}

function Save-RemovalPreflightReceipt($Session) {
    if($Session.ReceiptAttempted){throw 'Preflight receipt already attempted; never adopt/overwrite existing evidence.'}
    $Session.ReceiptAttempted=$true
    $snapshot=$Session.RetainedOwner.Shared.Snapshot
    $Session.SnapshotJson=ConvertTo-Json -InputObject $snapshot -Depth 16 -Compress
    $r=[ordered]@{SchemaVersion=1;NativeApproved=$false;Qualified=$false;ControllerPid=$Session.ControllerPid
        ControllerCreationUtcTicks=$Session.ControllerCreationUtcTicks;ControllerSession=$Session.ControllerSession
        ExecutionMode=$Session.ExecutionMode;CollectorPath=$Session.CollectorPath;CollectorSha256=$Session.CollectorSha256
        FiniteToolSha256=$Session.FiniteToolSha256;Snapshot=$snapshot}
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes((ConvertTo-Json -InputObject $r -Depth 18))
    if($bytes.Length -gt 1048576){throw 'Preflight receipt exceeds 1 MiB cap.'}
    $null=[IO.Directory]::CreateDirectory($Session.EvidenceDirectory)
    $path=Join-Path $Session.EvidenceDirectory 'preflight-receipt.json';Assert-FiniteToolPath $path $false
    $f=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$f.Write($bytes,0,$bytes.Length);$f.Flush($true)}finally{$f.Dispose()}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$hash=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}finally{$sha.Dispose()}
    if((Get-Item -LiteralPath $path).Length -ne $bytes.Length -or (Get-FileHash -LiteralPath $path).Hash -cne $hash){throw 'Preflight receipt readback differs.'}
    $Session.ReceiptPin=[pscustomobject]@{Path=$path;Bytes=$bytes.Length;Sha256=$hash}
    $Session.ReceiptPinJson=ConvertTo-Json -InputObject $Session.ReceiptPin -Compress
}

function Assert-RemovalPreflightSession {
    param([Parameter(Mandatory=$true,Position=0)]$Session)
    Assert-RemovalTaskHost;Assert-RemovalPreflightSessionIdentity $Session
    $owner=$Session.RetainedOwner
    if(-not$Session.Ready -or $null -eq $owner -or -not$owner.Completed -or -not$owner.Closed -or $owner.TimedOut -or
        $owner.Error -or $owner.Shared.Error -or $null -eq $Session.ReceiptPin){throw 'Original collector is not accepted, closed and evidenced.'}
    Assert-RemovalPreflightSnapshot $owner.Shared.Snapshot ([DateTimeOffset]::UtcNow) $Session.ControllerSession
    if((ConvertTo-Json -InputObject $owner.Shared.Snapshot -Depth 16 -Compress) -cne $Session.SnapshotJson){throw 'Original accepted snapshot changed.'}
    $pin=$Session.ReceiptPin
    if((ConvertTo-Json -InputObject $pin -Compress) -cne $Session.ReceiptPinJson -or
        $pin.Path -cne (Join-Path $Session.EvidenceDirectory 'preflight-receipt.json')){throw 'Original preflight receipt pin changed; no repinning.'}
    Assert-FiniteToolPath $pin.Path $false
    if((Get-Item -LiteralPath $pin.Path).Length -ne $pin.Bytes -or (Get-FileHash -LiteralPath $pin.Path).Hash -cne $pin.Sha256){throw 'Original preflight receipt changed.'}
}

function Observe-RemovalPreflightSession {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Session,[ValidateRange(1,120000)][int]$ObservationMilliseconds=25)
    $ErrorActionPreference='Stop';$owner=$Session.RetainedOwner;$needs=$true
    try {
        # Always settle retained work before any later source/identity refusal.
        if($null -ne $owner -and -not$owner.Closed){
            $r=Observe-RemovalCollectorWorker $owner -ObservationMilliseconds $ObservationMilliseconds
            if($owner.Completed){
                foreach($tool in $owner.ToolOwners){
                    if($tool.Closed){continue}
                    $null=Observe-FiniteTool $tool -ObservationMilliseconds ([Math]::Min(15000,$ObservationMilliseconds))
                    if($tool.ExitedConfirmed -and $tool.Copies.Count -eq 2 -and $tool.Flushes.Count -eq 2 -and
                        $null -ne $tool.CompletionObservedMilliseconds){
                        $all=$true;foreach($task in @($tool.Copies)+@($tool.Flushes)){
                            if(-not$task.IsCompleted){$all=$false}else{$null=$task.GetAwaiter().GetResult()}
                        }
                        if($all){Close-FiniteToolOwner $tool}
                    }
                }
                $r=Observe-RemovalCollectorWorker $owner -ObservationMilliseconds 1
                if(-not$r.NeedsObservation){
                    $needs=$false
                    if(-not$r.CollectionComplete){$Session.Error='Original collector failed/late/unknown: '+$r.Error}
                    Close-RemovalCollectorWorker $owner;$Session.ObservationComplete=$true
                }
            }
        }
        if($null -ne $owner -and $owner.Closed){$needs=$false}
        Assert-RemovalPreflightSessionIdentity $Session
        if($Session.Error){throw $Session.Error}
        if($Session.Ready){Assert-RemovalPreflightSession $Session}
        elseif($Session.ObservationComplete){
            Assert-RemovalPreflightSnapshot $owner.Shared.Snapshot ([DateTimeOffset]::UtcNow) $Session.ControllerSession
            Save-RemovalPreflightReceipt $Session;$Session.Ready=$true
            Assert-RemovalPreflightSession $Session
        }
    } catch {$Session.Ready=$false;if(-not$Session.Error){$Session.Error=$_.Exception.Message}}
    [pscustomobject]@{Ready=$Session.Ready;NeedsObservation=$needs;NativeApproved=$false;Qualified=$false
        ReceiptPin=$Session.ReceiptPin;Error=$Session.Error}
}
