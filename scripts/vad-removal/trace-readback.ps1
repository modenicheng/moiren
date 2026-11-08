#requires -Version 5.1
# Pure snapshot policy; obtaining actual native snapshots is a separate boundary.
Set-StrictMode -Version 2

function New-RemovalTraceReadback {
    param([Parameter(Mandatory=$true)]$Trace)
    [pscustomobject]@{Trace=$Trace;Initial=$null;Latest=$null;Final=$null;Attempted=$false;Invalid=$false
        FinalCountersKnown=$false;Lossless=$false;Error=$null}
}

function Assert-RemovalTraceSnapshot($Owner,$Snapshot,[switch]$Stopped) {
    if($null -eq $Snapshot){throw 'Missing complete native trace readback.'}
    $fields=@('ClientContext','LogFileMode','MaximumFileSize','EventsLost','LogBuffersLost','RealTimeBuffersLost')
    if(-not$Stopped){$fields+=@('SchemaVersion','QueryStatus','LoggerId')}
    foreach($field in $fields){
        $v=$Snapshot.$field
        if(($v -isnot [byte] -and $v -isnot [uint16] -and $v -isnot [int] -and $v -isnot [uint32] -and $v -isnot [long]) -or
            $v -lt 0 -or $v -gt [uint32]::MaxValue){throw ('Typed uint32 readback field required: '+$field)}
    }
    if((-not$Stopped-and($Snapshot.Complete -isnot [bool] -or -not $Snapshot.Complete -or $Snapshot.SchemaVersion -ne 1 -or
        $Snapshot.QueryStatus -ne 0 -or $Snapshot.LoggerId -eq 0))-or
        $Snapshot.SessionName -cne $Owner.Trace.Session -or
        $Snapshot.SessionGuid -isnot [Guid] -or $Snapshot.SessionGuid -eq [Guid]::Empty -or
        $Snapshot.Handle -isnot [uint64] -or $Snapshot.Handle -eq 0 -or
        $Snapshot.Handle -ne $Owner.Trace.Handle -or $Snapshot.SessionGuid -ne $Owner.Trace.SessionGuid -or
        $Snapshot.LogFileName -cne $Owner.Trace.EtlPath -or $Snapshot.ClientContext -ne 1 -or
        $Snapshot.LogFileMode -ne 1 -or $Snapshot.MaximumFileSize -ne 16){throw 'Native trace session/output/clock/mode/cap differs.'}
    if(-not$Stopped){
      $p=$Snapshot.Provider
      if($null -eq $p -or $p.Guid -isnot [Guid] -or $p.Guid -ne [Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2' -or
        $p.Enabled -isnot [bool] -or -not $p.Enabled -or $p.Kind -cne 'ClassicWpp' -or
        ($p.LoggerId -isnot [uint32] -and $p.LoggerId -isnot [int]) -or $p.LoggerId -ne $Snapshot.LoggerId -or
        ($p.Level -isnot [byte] -and $p.Level -isnot [int] -and $p.Level -isnot [uint32]) -or $p.Level -ne 4 -or
        ($p.EnableFlags -isnot [uint32] -and $p.EnableFlags -isnot [int]) -or $p.EnableFlags -ne 15){
        throw 'Exact classic WPP provider/session/level/flags not verified.'
      }
    }
    if($null -ne $Owner.Initial -and ($Snapshot.SessionGuid -ne $Owner.Initial.SessionGuid -or
        $Snapshot.Handle -ne $Owner.Initial.Handle -or (-not$Stopped-and$Snapshot.LoggerId -ne $Owner.Initial.LoggerId))){
        throw 'Native session identity changed; never adopt its replacement.'
    }
    if($null -ne $Owner.Latest){
        foreach($field in @('EventsLost','LogBuffersLost','RealTimeBuffersLost')){
            if($Snapshot.$field -lt $Owner.Latest.$field){throw ('Native loss counter decreased: '+$field)}
        }
    }
}

function Set-RemovalTraceReadbackError($Owner,[string]$Message) {
    $Owner.Invalid=$true;$Owner.Error=$Message;$Owner.Trace.SessionReadbackConfirmed=$false
    $Owner.Trace.Failed=$true
    if($Owner.Trace.Errors.Count -lt 16){$Owner.Trace.Errors.Add($Message)}
}

function Copy-RemovalTraceSnapshot($Snapshot) {
    # Freeze scalar copies. A caller mutating its detached snapshot later must
    # not replace the identity/counter baseline already independently observed.
    $copy=@{}
    foreach($name in @('SchemaVersion','Complete','QueryStatus','SessionName','SessionGuid','Handle','LoggerId','LogFileName',
        'ClientContext','LogFileMode','MaximumFileSize','EventsLost','LogBuffersLost','RealTimeBuffersLost')){$copy[$name]=$Snapshot.$name}
    $provider=@{};foreach($name in @('Guid','Enabled','LoggerId','Level','EnableFlags','Kind')){$provider[$name]=$Snapshot.Provider.$name}
    $copy.Provider=$provider
    return [pscustomobject]$copy
}

function Confirm-RemovalTraceReadback {
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[Parameter(Mandatory=$true,Position=1)]$Snapshot)
    if($Owner.Attempted -or $Owner.Invalid){throw 'Initial native readback already attempted/refused.'}
    $Owner.Attempted=$true
    try {
        if($Owner.Trace.TraceOwned -isnot [bool] -or -not $Owner.Trace.TraceOwned -or
            $Owner.Trace.TraceStopped -isnot [bool] -or $Owner.Trace.TraceStopped -or
            $Owner.Trace.OwnershipBasis -cne 'StartTraceHandleGuid' -or
            $Owner.Trace.StartStatus -isnot [uint32] -or $Owner.Trace.StartStatus -ne 0 -or
            $Owner.Trace.EnableStatus -isnot [uint32] -or $Owner.Trace.EnableStatus -ne 0 -or
            $Owner.Trace.Handle -isnot [uint64] -or $Owner.Trace.Handle -eq 0 -or
            $Owner.Trace.SessionGuid -isnot [Guid] -or $Owner.Trace.SessionGuid -eq [Guid]::Empty){
            throw 'No confirmed retained StartTrace handle/GUID and provider-enable provenance.'
        }
        Assert-RemovalTraceSnapshot $Owner $Snapshot
        $Owner.Initial=Copy-RemovalTraceSnapshot $Snapshot;$Owner.Latest=Copy-RemovalTraceSnapshot $Snapshot
        $Owner.Trace.SessionReadbackConfirmed=$true
    } catch {Set-RemovalTraceReadbackError $Owner $_.Exception.Message;throw}
}

function Observe-RemovalTraceReadback {
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[Parameter(Mandatory=$true,Position=1)]$Snapshot)
    try {
        if($Owner.Invalid -or $null -eq $Owner.Initial -or $Owner.FinalCountersKnown -or $Owner.Trace.TraceStopped){throw 'No unchanged live initial native trace binding.'}
        Assert-RemovalTraceSnapshot $Owner $Snapshot
        $Owner.Latest=Copy-RemovalTraceSnapshot $Snapshot
    } catch {Set-RemovalTraceReadbackError $Owner $_.Exception.Message;throw}
}

function Test-RemovalTraceCaptureAdmission($Owner) {
    return ($Owner.Invalid -is [bool] -and -not $Owner.Invalid -and $null -ne $Owner.Initial -and
        $Owner.Trace.TraceOwned -is [bool] -and $Owner.Trace.TraceOwned -and
        $Owner.Trace.TraceStopped -is [bool] -and -not $Owner.Trace.TraceStopped -and
        $Owner.Trace.SessionReadbackConfirmed -is [bool] -and $Owner.Trace.SessionReadbackConfirmed -and
        -not $Owner.Trace.Failed)
}

function Complete-RemovalTraceLoss {
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[Parameter(Mandatory=$true,Position=1)]$Receipt)
    try {
        if($Owner.Invalid -or $null -eq $Owner.Initial -or $Owner.FinalCountersKnown -or
            $Owner.Trace.TraceStopped -isnot [bool] -or -not $Owner.Trace.TraceStopped -or
            -not[object]::ReferenceEquals($Receipt,$Owner.Trace.StopReceipt) -or
            $Receipt.Operation -cne 'Stop' -or -not[object]::ReferenceEquals($Receipt.Owner,$Owner.Trace) -or
            $Receipt.Status -isnot [uint32] -or $Receipt.Status -ne 0 -or
            $Receipt.OwnedHandle -isnot [uint64] -or $Receipt.OwnedHandle -ne $Owner.Initial.Handle -or
            $Receipt.SessionGuid -isnot [Guid] -or $Receipt.SessionGuid -ne $Owner.Initial.SessionGuid -or
            $Receipt.SessionName -cne $Owner.Trace.Session -or $Receipt.EtlPath -cne $Owner.Trace.EtlPath -or
            $Receipt.StoppedConfirmed -isnot [bool] -or -not$Receipt.StoppedConfirmed -or
            $Receipt.FinalCountersKnown -isnot [bool] -or -not$Receipt.FinalCountersKnown){
            throw 'Final counters require exact successful owned STOP receipt and complete returned properties.'
        }
        $Snapshot=$Receipt.Snapshot
        Assert-RemovalTraceSnapshot $Owner $Snapshot -Stopped
        $final=@{}
        foreach($field in @('SessionName','SessionGuid','Handle','LogFileName','ClientContext','LogFileMode','MaximumFileSize','EventsLost','LogBuffersLost','RealTimeBuffersLost')){$final[$field]=$Snapshot.$field}
        $Owner.Final=[pscustomobject]$final
        $Owner.Trace.RawTraceLoss=[pscustomobject]@{EventsLost=$Snapshot.EventsLost;LogBuffersLost=$Snapshot.LogBuffersLost
            RealTimeBuffersLost=$Snapshot.RealTimeBuffersLost}
        $Owner.FinalCountersKnown=$true
        $Owner.Lossless=($Snapshot.EventsLost -eq 0 -and $Snapshot.LogBuffersLost -eq 0 -and $Snapshot.RealTimeBuffersLost -eq 0)
    } catch {Set-RemovalTraceReadbackError $Owner $_.Exception.Message;throw}
}
