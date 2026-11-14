#requires -Version 5.1
# Definitions only. Loading this file launches no tools and opens no ETW session.
Set-StrictMode -Version 2

function Assert-RemovalTracePath([string]$Path) {
    if($Path -notmatch '^[A-Za-z]:\\' -or $Path -match '[\x00-\x1f"*?]' -or
        $Path.Substring(2).Contains(':') -or [IO.Path]::GetFullPath($Path) -cne $Path){
        throw 'Canonical local absolute path required.'
    }
    $cursor=$Path
    while($cursor){
        if(Test-Path -LiteralPath $cursor){
            $item=Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
            if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Reparse path refused.'}
            if($cursor -cne $Path -and -not $item.PSIsContainer){throw 'Non-directory ancestor.'}
        }
        $cursor=[IO.Path]::GetDirectoryName($cursor)
    }
}

function Assert-RemovalTraceFinitePin([string]$Path) {
    Assert-RemovalTracePath $Path
    if((Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash -cne
        'C3621DC9C881519006E01619D2C65E053C3A1F0FBFC100C0E247E3064D16651B'){
        throw 'Frozen finite-tool hash mismatch.'
    }
}

function New-RemovalTraceBoundary {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory=$true)][string]$Session,
        [Parameter(Mandatory=$true)][string]$EtlPath,
        [Parameter(Mandatory=$true)][string]$EvidenceDirectory,
        [Parameter(Mandatory=$true)][string]$LogmanPath,
        [Parameter(Mandatory=$true)][ValidatePattern('\A[A-Fa-f0-9]{64}\z')][string]$LogmanSha256,
        [Parameter(Mandatory=$true)][string]$TracelogPath,
        [Parameter(Mandatory=$true)][ValidatePattern('\A[A-Fa-f0-9]{64}\z')][string]$TracelogSha256,
        [string]$FiniteToolPath=(Join-Path $PSScriptRoot 'finite-tool.ps1'),
        [hashtable]$ToolBoundary
    )
    $ErrorActionPreference='Stop'
    if($Session -cnotmatch '\AMoirenRemoval613[A-Za-z0-9_-]{1,64}\z'){throw 'Unsafe trace session namespace.'}
    foreach($path in @($EtlPath,$EvidenceDirectory,$LogmanPath,$TracelogPath)){Assert-RemovalTracePath $path}
    if([IO.Path]::GetFileName($LogmanPath) -ine 'logman.exe' -or
        [IO.Path]::GetFileName($TracelogPath) -ine 'tracelog.exe'){throw 'Exact logman/tracelog executable paths required.'}
    if([IO.Path]::GetExtension($EtlPath) -ine '.etl' -or
        -not $EtlPath.StartsWith(($EvidenceDirectory.TrimEnd('\')+'\'),[StringComparison]::OrdinalIgnoreCase) -or
        (Test-Path -LiteralPath $EtlPath)){throw 'Fresh contained ETL path required.'}
    if((Test-Path -LiteralPath $EvidenceDirectory) -and
        -not (Test-Path -LiteralPath $EvidenceDirectory -PathType Container)){throw 'Evidence root is not a directory.'}
    foreach($step in @('inventory','start','loss','stop')){
        if(Test-Path -LiteralPath (Join-Path $EvidenceDirectory ('trace-'+$step))){throw 'Create-new per-step tool output required.'}
    }
    Assert-RemovalTraceFinitePin $FiniteToolPath
    if($null -ne $ToolBoundary){
        foreach($name in @('Start','Observe','Read')){
            if($ToolBoundary[$name] -isnot [scriptblock]){throw 'Incomplete test tool boundary.'}
        }
    }
    [pscustomobject]@{
        Session=$Session;EtlPath=$EtlPath;EvidenceDirectory=$EvidenceDirectory
        LogmanPath=$LogmanPath;LogmanSha256=$LogmanSha256
        TracelogPath=$TracelogPath;TracelogSha256=$TracelogSha256
        FiniteToolPath=$FiniteToolPath;ToolBoundary=$ToolBoundary
        InventoryAttempted=$false;InventoryConfirmed=$false;StartAttempted=$false
        LossAttempted=$false;StopAttempted=$false;TraceOwned=$false;TraceStopped=$false
        RawTraceLoss='unknown';Qualified=$false;NativeApproved=$false;Failed=$false
        OwnershipBasis=$null;SessionReadbackConfirmed=$false
        ToolOwners=New-Object Collections.Generic.List[object]
        Steps=New-Object Collections.Generic.List[object]
        Errors=New-Object Collections.Generic.List[string]
    }
}

function Add-RemovalTraceError($Trace,[string]$Message) {
    $Trace.Failed=$true
    if($Trace.Errors.Count -lt 16 -and -not $Trace.Errors.Contains($Message)){$Trace.Errors.Add($Message)}
}

function Assert-RemovalTraceNativeApproval {
    # Callability is not authorization. The future combined runner/approval
    # contract has not been reviewed; no mutable field may unlock real tools.
    # Mock/harmless composition replaces the explicit ToolBoundary, not this
    # production launcher. Loading these definitions remains side-effect free.
    throw 'Trace preparation is not approved for native tool invocation.'
}

function Start-RemovalTraceFiniteTool($Trace,$Request) {
    Assert-RemovalTraceNativeApproval
    Assert-RemovalTraceFinitePin $Trace.FiniteToolPath
    . $Trace.FiniteToolPath
    Start-FiniteTool -Executable $Request.Executable -ExpectedSha256 $Request.ExpectedSha256 `
        -ArgumentList $Request.ArgumentList -OutputDirectory $Request.OutputDirectory `
        -TimeoutSeconds $Request.TimeoutSeconds -MaxLogBytes $Request.MaxLogBytes
}

function Observe-RemovalTraceFiniteTool($Trace,$Owner) {
    Assert-RemovalTraceFinitePin $Trace.FiniteToolPath
    . $Trace.FiniteToolPath
    Observe-FiniteTool -Owner $Owner
}

function Read-RemovalTraceOutput($Receipt) {
    # Read raw bytes, not Get-Content's locale-dependent default encoding.
    # Finite-tool intentionally retains its write handles after a stable receipt.
    # Our access stays read-only; sharing must permit that already-open writer.
    $stream=[IO.File]::Open($Receipt.StdoutPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    try {
        $length=$stream.Length
        if($length -le 0 -or $length -gt 1048576){throw 'Inventory output empty/over cap.'}
        $bytes=[byte[]]::new([int]$length);$offset=0
        while($offset -lt $bytes.Length){
            $read=$stream.Read($bytes,$offset,$bytes.Length-$offset)
            if($read -eq 0){throw 'Inventory file changed/short read.'}
            $offset+=$read
        }
        if($stream.ReadByte() -ne -1 -or $stream.Length -ne $length){throw 'Inventory output changed.'}
        return ,$bytes
    } finally {$stream.Dispose()}
}

function Convert-RemovalTraceInventory([byte[]]$Bytes,[string]$Session) {
    if($null -eq $Bytes -or $Bytes.Length -eq 0 -or $Bytes.Length -gt 1048576){throw 'Inventory bytes empty/over cap.'}
    if($Bytes.Length -ge 2 -and $Bytes[0] -eq 255 -and $Bytes[1] -eq 254){
        $text=[Text.UnicodeEncoding]::new($false,$true,$true).GetString($Bytes,2,$Bytes.Length-2)
    } elseif($Bytes.Length -ge 2 -and $Bytes[0] -eq 254 -and $Bytes[1] -eq 255){
        $text=[Text.UnicodeEncoding]::new($true,$true,$true).GetString($Bytes,2,$Bytes.Length-2)
    } elseif($Bytes.Length -ge 3 -and $Bytes[0] -eq 239 -and $Bytes[1] -eq 187 -and $Bytes[2] -eq 191){
        $text=[Text.UTF8Encoding]::new($false,$true).GetString($Bytes,3,$Bytes.Length-3)
    } else {
        # ANSI/OEM/UTF-8 all preserve this restricted ASCII session name.
        # Do not interpret localized labels or infer counters from this view.
        $text=[Text.Encoding]::GetEncoding(28591).GetString($Bytes)
    }
    if([string]::IsNullOrWhiteSpace($text) -or $text.Contains([string][char]0)){throw 'Ambiguous inventory encoding/content.'}
    $pattern='(?<![A-Za-z0-9_-])'+[regex]::Escape($Session)+'(?![A-Za-z0-9_-])'
    if([regex]::IsMatch($text,$pattern,([Text.RegularExpressions.RegexOptions]::IgnoreCase -bor
        [Text.RegularExpressions.RegexOptions]::CultureInvariant))){
        # Positive presence refuses even when localized formatting is unknown.
        return [pscustomobject]@{Sessions=@($Session);Format='PresenceOnly';SessionCount=$null}
    }
    # Negative substring search is NOT an absence proof. Admit only this complete
    # table shape, including its final success footer and every intervening row.
    # Other locales/layouts refuse pending a separately reviewed parser fixture;
    # no live target format or semantic completeness has been qualified here.
    $lines=@($text -split '\r\n|\n|\r'|ForEach-Object{$_.Trim()}|Where-Object{$_ -ne ''})
    if($lines.Count -lt 3 -or $lines[0] -cnotmatch '\AData Collector Set {2,}Type {2,}Status\z' -or
        $lines[1] -cnotmatch '\A-{3,}\z' -or $lines[$lines.Count-1] -cne 'The command completed successfully.'){
        throw 'Unrecognized/truncated inventory; absence remains unknown.'
    }
    $names=@{}
    for($i=2;$i -lt $lines.Count-1;$i++){
        $row=[regex]::Match($lines[$i],'\A(.+?) {2,}Trace {2,}Running\z')
        if(-not $row.Success){throw 'Malformed inventory row; absence remains unknown.'}
        $name=$row.Groups[1].Value.Trim()
        if(-not $name -or $name.Length -gt 1024 -or $names.ContainsKey($name)){
            throw 'Unknown/duplicate inventory session identity.'
        }
        $names[$name]=$true
    }
    return [pscustomobject]@{Sessions=@();Format='EnglishLogmanQueryV1';SessionCount=$names.Count}
}

function Test-RemovalTraceToolReceipt($Receipt,$Owner) {
    # Do not coerce null/"0"/"False" into a successful native result.
    try {
        return ($null -ne $Owner -and $Owner.IdentityConfirmed -is [bool] -and $Owner.IdentityConfirmed -and
            $Owner.ProcessId -is [int] -and $Owner.ProcessId -gt 0 -and
            $Owner.CreationUtcTicks -is [long] -and $Owner.CreationUtcTicks -gt 0 -and
            [object]::ReferenceEquals($Receipt.Owner,$Owner) -and
            $Receipt.ProcessId -is [int] -and $Receipt.ProcessId -eq $Owner.ProcessId -and
            $Receipt.CreationUtcTicks -is [long] -and $Receipt.CreationUtcTicks -eq $Owner.CreationUtcTicks -and
            $Receipt.Success -is [bool] -and $Receipt.Success -and
            $Receipt.ExitedConfirmed -is [bool] -and $Receipt.ExitedConfirmed -and
            $Receipt.ExitCode -is [int] -and $Receipt.ExitCode -eq 0 -and
            $Receipt.TimedOut -is [bool] -and -not $Receipt.TimedOut -and
            $Receipt.OutputStable -is [bool] -and $Receipt.OutputStable -and
            $Receipt.NeedsObservation -is [bool] -and -not $Receipt.NeedsObservation -and
            $Receipt.LogLimitExceeded -is [bool] -and -not $Receipt.LogLimitExceeded -and
            $Receipt.Errors -is [Array] -and $Receipt.Errors.Count -eq 0)
    } catch {return $false}
}

function Invoke-RemovalTraceStep {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Trace,
        [Parameter(Mandatory=$true,Position=1)][string]$Step)
    $ErrorActionPreference='Stop'
    try {
        switch -CaseSensitive ($Step){
            'inventory' {
                if($Trace.InventoryAttempted -or $Trace.StartAttempted -or $Trace.Failed){throw 'Inventory unavailable/already attempted.'}
                $Trace.InventoryAttempted=$true
                $argv=@('query','-ets');break
            }
            'start' {
                if(-not $Trace.InventoryConfirmed -or $Trace.StartAttempted -or $Trace.Failed){throw 'Start needs clean inventory and one unused attempt.'}
                $Trace.StartAttempted=$true
                $argv=@('create','trace',$Trace.Session,'-o',$Trace.EtlPath,'-p',
                    '{7DC75BA9-8274-4EAE-A42D-B4C74E0209D2}','0xf','4','-f','bin','-max','16','-ct','perf','-ets');break
            }
            'loss' {
                if(-not $Trace.TraceOwned -or $Trace.LossAttempted -or $Trace.StopAttempted){throw 'Loss needs confirmed-owned, unstopped session and one unused attempt.'}
                $Trace.LossAttempted=$true;$argv=@('-q',$Trace.Session);break
            }
            'stop' {
                if(-not $Trace.TraceOwned -or $Trace.StopAttempted){throw 'Stop needs confirmed-owned session and one unused attempt.'}
                $Trace.StopAttempted=$true;$argv=@('stop',$Trace.Session,'-ets');break
            }
            default {throw 'Unknown trace step.'}
        }
    } catch {Add-RemovalTraceError $Trace $_.Exception.Message;throw}
    $entry=[pscustomobject]@{
        Step=$Step;ToolOwner=$null;ToolReceipt=$null;Request=$null;DispatchUnknown=$false
        ExitCode=$null;TimedOut=$null;Error=$null;Success=$false;Sessions=@()
        InventoryFormat=$null;InventorySessionCount=$null
    }
    $Trace.Steps.Add($entry)
    try {
        $loss=($Step -ceq 'loss')
        $entry.Request=[pscustomobject]@{
            Step=$Step
            Executable=$(if($loss){$Trace.TracelogPath}else{$Trace.LogmanPath})
            ExpectedSha256=$(if($loss){$Trace.TracelogSha256}else{$Trace.LogmanSha256})
            ArgumentList=$argv;OutputDirectory=(Join-Path $Trace.EvidenceDirectory ('trace-'+$Step))
            TimeoutSeconds=15;MaxLogBytes=1048576
        }
        if($Step -ceq 'start'){
            Assert-RemovalTracePath $Trace.EtlPath
            if(Test-Path -LiteralPath $Trace.EtlPath){throw 'ETL appeared after inventory; no overwrite.'}
        }
        # A dispatch exception has no safe retry. If finite-tool returned an
        # unresolved object, retain it before any observation can throw.
        $entry.DispatchUnknown=$true
        $owner=if($null -ne $Trace.ToolBoundary){&$Trace.ToolBoundary.Start $entry.Request}
            else{Start-RemovalTraceFiniteTool $Trace $entry.Request}
        $entry.ToolOwner=$owner
        if($null -eq $owner){throw 'Tool dispatch returned no exact owner.'}
        $Trace.ToolOwners.Add($owner)
        $receipt=if($null -ne $Trace.ToolBoundary){&$Trace.ToolBoundary.Observe $owner $Trace}
            else{Observe-RemovalTraceFiniteTool $Trace $owner}
        $entry.ToolReceipt=$receipt
        $entry.ExitCode=$receipt.ExitCode;$entry.TimedOut=$receipt.TimedOut
        if(-not (Test-RemovalTraceToolReceipt $receipt $owner)){throw 'Tool failed/late/unknown; exact owner retained.'}
        $entry.DispatchUnknown=$false
        if($Step -ceq 'start'){
            # Only a complete successful exact create establishes ownership.
            # Inventory/query never adopts an existing session. Publish before
            # any later caller receipt write; a failed write cannot orphan it.
            $Trace.TraceOwned=$true
            $Trace.OwnershipBasis='ExactCreateExitZero'
            # This is create-command provenance, not a queried session/ETL/
            # provider readback or assurance against another controller's races.
            $Trace.SessionReadbackConfirmed=$false
        } elseif($Step -ceq 'stop'){$Trace.TraceStopped=$true}
        elseif($Step -ceq 'inventory'){
            $bytes=if($null -ne $Trace.ToolBoundary){&$Trace.ToolBoundary.Read $receipt}
                else{Read-RemovalTraceOutput $receipt}
            $inventory=Convert-RemovalTraceInventory $bytes $Trace.Session
            $entry.Sessions=$inventory.Sessions
            $entry.InventoryFormat=$inventory.Format;$entry.InventorySessionCount=$inventory.SessionCount
            if($entry.Sessions.Count){throw 'Exact intended session already exists; do not adopt/stop it.'}
            $Trace.InventoryConfirmed=$true
        }
        $entry.Success=$true
    } catch {
        $entry.Error='Trace '+$Step+' failed/unknown; no retry, retain exact tool owners.'
        Add-RemovalTraceError $Trace $entry.Error
    }
    # No owner is disposed automatically; caller explicitly observes/closes the
    # original finite-tool objects. Loss remains raw/unknown even on exit 0.
    return $entry
}
