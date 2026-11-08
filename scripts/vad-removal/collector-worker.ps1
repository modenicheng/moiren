#requires -Version 5.1
# Managed runspace owner, not process isolation or native acceptance.
Set-StrictMode -Version 2

function New-CollectorDiagnosticDrain($PowerShell) {
    if(-not('Moiren.CollectorDrainV1' -as [type])){
        # Compiled callbacks do not need a PowerShell runspace on pipeline
        # threads. ReadAll drops stream objects immediately; retain only count.
        $references=@([PowerShell].Assembly.Location)
        if($PSVersionTable.PSEdition -eq 'Core'){$references+=Join-Path $PSHOME 'ref\System.Threading.dll'}
        Add-Type -ReferencedAssemblies $references -TypeDefinition @'
using System;
using System.Management.Automation;
namespace Moiren {
    public sealed class CollectorDrainV1 {
        private readonly object gate = new object();
        private long count;
        public long EmittedCount { get { lock(gate) { return count; } } }
        private void Attach<T>(PSDataCollection<T> stream) {
            stream.DataAdded += delegate(object sender, DataAddedEventArgs args) {
                var rows = stream.ReadAll();
                lock(gate) { count += rows.Count; }
            };
        }
        public CollectorDrainV1(PowerShell owner) {
            Attach(owner.Streams.Error); Attach(owner.Streams.Warning);
            Attach(owner.Streams.Information); Attach(owner.Streams.Verbose);
            Attach(owner.Streams.Debug); Attach(owner.Streams.Progress);
        }
    }
}
'@
    }
    return [Moiren.CollectorDrainV1]::new($PowerShell)
}

function Assert-CollectorInput([string]$Path,[string]$Hash) {
    if($Path -notmatch '^[A-Za-z]:\\' -or [IO.Path]::GetFullPath($Path) -cne $Path -or
        $Path -match '[\x00\r\n"*?]' -or $Hash -cnotmatch '^[A-Fa-f0-9]{64}$'){throw 'Canonical pinned local input required.'}
    $cursor=$Path
    while($cursor){
        if(Test-Path -LiteralPath $cursor){
            $item=Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
            if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Reparse input refused.'}
        }
        $cursor=[IO.Path]::GetDirectoryName($cursor)
    }
    if(-not(Test-Path -LiteralPath $Path -PathType Leaf) -or (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ine $Hash){throw 'Collector/helper input hash differs.'}
}

function Start-RemovalCollectorWorker {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$FiniteToolPath,
        [Parameter(Mandatory=$true)][string]$FiniteToolSha256,
        [Parameter(Mandatory=$true)][string]$CollectorPath,
        [Parameter(Mandatory=$true)][string]$CollectorSha256,
        [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
        [string]$ReadToolPinsJson,
        [ValidateRange(0.01,120)][double]$TimeoutSeconds=120)
    $ErrorActionPreference='Stop'
    Assert-CollectorInput $FiniteToolPath $FiniteToolSha256
    Assert-CollectorInput $CollectorPath $CollectorSha256
    if($ReadToolPinsJson.Length -gt 16384){throw 'Read-tool pin configuration exceeds bounded input cap.'}
    if($EvidenceDirectory -notmatch '^[A-Za-z]:\\' -or [IO.Path]::GetFullPath($EvidenceDirectory) -cne $EvidenceDirectory -or
        $EvidenceDirectory -match '[\x00\r\n"*?]' -or (Test-Path -LiteralPath $EvidenceDirectory)){throw 'Fresh canonical evidence path required.'}
    $shared=[hashtable]::Synchronized(@{ToolOwners=New-Object Collections.Generic.List[object];Snapshot=$null;Error=$null;OutputCount=0})
    $owner=[pscustomobject]@{PowerShell=$null;Invocation=$null;Shared=$shared;ToolOwners=$shared.ToolOwners;Diagnostics=$null
        LaunchAttempted=$false;Completed=$false;Closed=$false;TimedOut=$false;CompletionObservedMilliseconds=$null
        EndInvoked=$false;Error=$null;Clock=[Diagnostics.Stopwatch]::StartNew()
        DeadlineMilliseconds=[long][Math]::Ceiling($TimeoutSeconds*1000);EvidenceDirectory=$EvidenceDirectory}
    try {
        $owner.PowerShell=[PowerShell]::Create()
        $owner.Diagnostics=New-CollectorDiagnosticDrain $owner.PowerShell
        # The one owned runspace retains live .NET tool handles in shared memory.
        # Parent observation never calls Stop/Dispose on an unfinished pipeline.
        $script=@'
param($state,$finite,$finiteHash,$collector,$collectorHash,$directory,$readToolPinsJson)
$ErrorActionPreference='Stop'
$WarningPreference='Stop'
$InformationPreference='SilentlyContinue'
$VerbosePreference='SilentlyContinue'
$DebugPreference='SilentlyContinue'
$ProgressPreference='SilentlyContinue'
Set-StrictMode -Version 2
try {
    if((Get-FileHash -LiteralPath $finite -Algorithm SHA256).Hash -ine $finiteHash -or
       (Get-FileHash -LiteralPath $collector -Algorithm SHA256).Hash -ine $collectorHash){throw 'Worker input changed before execution.'}
    $null = . $finite | ForEach-Object { throw 'Unexpected top-level helper output.' }
    $null = . $collector | ForEach-Object { throw 'Unexpected top-level collector output.' }
    function Assert-SnapshotSize($Value,[int]$Depth,$Budget){
        $Budget.Nodes++
        if($Depth -gt 16 -or $Budget.Nodes -gt 8192){throw 'Snapshot depth/item cap exceeded.'}
        if($null -eq $Value){return}
        if($Value -is [string]){
            $Budget.Characters+=$Value.Length
            if($Value.Length -gt 4096 -or $Budget.Characters -gt 262144){throw 'Snapshot text cap exceeded.'}
            return
        }
        if($Value -is [bool] -or $Value -is [ValueType]){return}
        if($Value -is [Collections.IDictionary]){
            foreach($key in $Value.Keys){
                if($key -isnot [string]){throw 'Snapshot map key must be text.'}
                Assert-SnapshotSize $key ($Depth+1) $Budget
                Assert-SnapshotSize $Value[$key] ($Depth+1) $Budget
            }
            return
        }
        if($Value -is [Collections.IEnumerable]){
            foreach($item in $Value){Assert-SnapshotSize $item ($Depth+1) $Budget}
            return
        }
        if($Value -is [pscustomobject]){
            foreach($property in $Value.PSObject.Properties){Assert-SnapshotSize $property.Name ($Depth+1) $Budget;Assert-SnapshotSize $property.Value ($Depth+1) $Budget}
            return
        }
        throw 'Unsupported snapshot object type.'
    }
    # Stream validation rejects the first unexpected value rather than building
    # an unbounded @() array before discovering malformed collector output.
    $candidate=$null
    $arguments=@{EvidenceDirectory=$directory;ToolOwners=$state.ToolOwners}
    if($readToolPinsJson){
        $pins=ConvertFrom-Json -InputObject $readToolPinsJson
        if($null -ne $pins){$arguments.ReadToolPins=$pins}
    }
    Read-RemovalPreflight @arguments | ForEach-Object {
        $state.OutputCount++
        if($state.OutputCount -ne 1 -or $_ -isnot [Collections.IDictionary]){throw 'Collector returned missing/multiple/non-dictionary snapshot.'}
        $candidate=$_
    }
    if($state.OutputCount -ne 1){throw 'Collector returned no snapshot.'}
    $snapshot=$candidate
    if($snapshot.schema_version -ne 1 -or $snapshot.NativeTestStarted -isnot [bool] -or $snapshot.NativeTestStarted -or
       $snapshot.Qualified -isnot [bool] -or $snapshot.Qualified){throw 'Malformed collector activity/acceptance flags.'}
    Assert-SnapshotSize $snapshot 0 @{Nodes=0;Characters=0}
    $state.Snapshot=$snapshot
} catch { $message=$_.Exception.Message;$state.Error=$message.Substring(0,[Math]::Min(4096,$message.Length)) }
'@
        $null=$owner.PowerShell.AddScript($script).AddArgument($shared).AddArgument($FiniteToolPath).AddArgument($FiniteToolSha256).AddArgument($CollectorPath).AddArgument($CollectorSha256).AddArgument($EvidenceDirectory).AddArgument($ReadToolPinsJson)
        $owner.LaunchAttempted=$true
        $owner.Invocation=$owner.PowerShell.BeginInvoke()
    } catch {$owner.Error='Worker dispatch failed/unknown; retain original owner.'}
    return $owner
}

function Observe-RemovalCollectorWorker {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[ValidateRange(0,120000)][int]$ObservationMilliseconds=0)
    if($Owner.Closed){throw 'Collector worker already closed.'}
    if(-not $Owner.Completed -and $null -ne $Owner.Invocation){
        $end=if($ObservationMilliseconds -gt 0){$Owner.Clock.ElapsedMilliseconds+$ObservationMilliseconds}else{$Owner.DeadlineMilliseconds}
        while(-not $Owner.Invocation.IsCompleted){
            $left=$end-$Owner.Clock.ElapsedMilliseconds
            if($left -le 0){break}
            $null=$Owner.Invocation.AsyncWaitHandle.WaitOne([int][Math]::Min(25,$left))
        }
        if($Owner.Invocation.IsCompleted){
            $Owner.EndInvoked=$true
            try {
                $unexpected=$Owner.PowerShell.EndInvoke($Owner.Invocation)
                if($null -ne $unexpected -and $unexpected.Count){$Owner.Error='Unexpected worker pipeline output.'}
            }catch{$Owner.Error='Worker pipeline completion failed.'}
            if(($Owner.PowerShell.HadErrors -or $Owner.PowerShell.Streams.Error.Count -or $Owner.Diagnostics.EmittedCount) -and $null -eq $Owner.Shared.Error){$Owner.Error='Worker emitted diagnostics; snapshot cannot authorize follow-up.'}
            $Owner.Completed=$true;$Owner.CompletionObservedMilliseconds=$Owner.Clock.ElapsedMilliseconds
        }
        $observed=if($Owner.Completed){$Owner.CompletionObservedMilliseconds}else{$Owner.Clock.ElapsedMilliseconds}
        if($observed -ge $Owner.DeadlineMilliseconds){$Owner.TimedOut=$true}
    }
    # Never enumerate the mutable inner list before worker completion.
    $innerClosed=$Owner.Completed -and @($Owner.ToolOwners|Where-Object{-not $_.Closed}).Count -eq 0
    $errorText=if($null -ne $Owner.Error){$Owner.Error}else{$Owner.Shared.Error}
    $success=$Owner.Completed -and $innerClosed -and -not $Owner.TimedOut -and $null -eq $errorText -and $null -ne $Owner.Shared.Snapshot
    [pscustomobject]@{Owner=$Owner;Completed=$Owner.Completed;CollectionComplete=$success;TimedOut=$Owner.TimedOut
        InnerOwnersClosed=$innerClosed;NeedsObservation=(-not $Owner.Completed -or -not $innerClosed)
        Snapshot=$Owner.Shared.Snapshot;Error=$errorText;NativeApproved=$false;Qualified=$false}
}

function Close-RemovalCollectorWorker {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    if($Owner.Closed){return}
    if(-not $Owner.Completed -or @($Owner.ToolOwners|Where-Object{-not $_.Closed}).Count){throw 'Worker/inner owners unresolved; preserve invocation and exact handles.'}
    $Owner.Invocation.AsyncWaitHandle.Dispose()
    $Owner.PowerShell.Dispose();$Owner.Closed=$true
}
