#requires -Version 5.1
# The bootstrap holds the independently pinned manifest and complete source set
# before importing this library, and retains those handles throughout Invoke.
# This wrapper owns one exact child and both pipes; never elevate, kill, detach,
# relaunch or turn closed failure evidence into native/release qualification.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
. (Join-Path $PSScriptRoot 'delivery-finalize.ps1')
$script:RemovalDiagnosticEntrySourcePath=$PSCommandPath
if($null-eq(Get-Variable RemovalDiagnosticEntryOwners -Scope Script -ErrorAction SilentlyContinue)){
    $script:RemovalDiagnosticEntryOwners=New-Object Collections.Generic.List[object]
}

function Assert-RemovalDiagnosticEntryTarget([string]$RunDirectory){
    if([Environment]::MachineName-cne'LAPTOP-NP49RG6H'-or
        [Security.Principal.WindowsIdentity]::GetCurrent().User.Value-cne'S-1-5-21-3080296863-1242338503-1346362208-1001'){
        throw 'Wrong diagnostic entry host/SID; no files or process dispatch.'
    }
    if(-not([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Already elevated diagnostic entry required; no automatic elevation.'}
    if($RunDirectory-cnotmatch'\AD:\\MoirenTest\\'){throw 'Diagnostic entry run must belong to the target test workspace.'}
}

function Get-RemovalDiagnosticEntryScript([string]$SourceDirectory){Join-Path $SourceDirectory 'invoke-diagnostic.ps1'}

function Add-RemovalDiagnosticEntryError($Owner,[string]$Message){
    if($Owner.Errors.Count-lt32-and-not$Owner.Errors.Contains($Message)){$Owner.Errors.Add($Message)}
}

function Get-RemovalDiagnosticEntryStreamPin([string]$Path,$Stream){
    $Stream.Position=0;$sha=[Security.Cryptography.SHA256]::Create()
    try{[pscustomobject]@{Path=$Path;Bytes=[long]$Stream.Length;Sha256=[BitConverter]::ToString($sha.ComputeHash($Stream)).Replace('-','')}}
    finally{$sha.Dispose()}
}

function Open-RemovalDiagnosticEntryFile([string]$Path,[long]$MaximumBytes,$Handles){
    Assert-FiniteToolPath $Path $false
    $stream=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $Handles.Add($stream)
    if($stream.Length-gt$MaximumBytes){throw 'Entry file exceeds bounded byte cap.'}
    [pscustomobject]@{Pin=(Get-RemovalDiagnosticEntryStreamPin $Path $stream);Stream=$stream}
}

function Read-RemovalDiagnosticEntryJson($Stream){
    if($Stream.Length-le0-or$Stream.Length-gt1048576){throw 'Entry JSON must be nonempty and at most 1 MiB.'}
    $Stream.Position=0;$bytes=New-Object byte[] ([int]$Stream.Length);$offset=0
    while($offset-lt$bytes.Length){$n=$Stream.Read($bytes,$offset,$bytes.Length-$offset);if($n-le0){throw 'Entry JSON became short.'};$offset+=$n}
    $json=[Text.UTF8Encoding]::new($false,$true).GetString($bytes)
    # One whole JSON object only, no last-line extraction or duplicate-key merge.
    ConvertFrom-RemovalDeliveryJson $json
}

function Write-RemovalDiagnosticEntryJson([string]$Path,$Value){
    $bytes=[Text.UTF8Encoding]::new($false,$true).GetBytes((ConvertTo-Json -InputObject $Value -Depth 24 -Compress))
    $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read)
    try{
        $stream.Write($bytes,0,$bytes.Length);$stream.Flush($true)
        $pin=Get-RemovalDiagnosticEntryStreamPin $Path $stream
        $sha=[Security.Cryptography.SHA256]::Create()
        try{$expected=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}finally{$sha.Dispose()}
        if($pin.Bytes-ne$bytes.Length-or$pin.Sha256-cne$expected){throw 'Entry receipt durable readback differs.'}
        return $pin
    }finally{$stream.Dispose()}
}

function Assert-RemovalDiagnosticEntryLocalAck($Owner){
    $output=Read-RemovalDiagnosticEntryJson $Owner.Files[0]
    Assert-RemovalDeliveryObject $output
    if($output.Qualified-isnot[bool]-or$output.Qualified){throw 'Entry stdout qualification flag differs.'}
    $published=$output.LocalAcknowledgement
    Assert-RemovalDeliveryPin $published 1048576
    if($published.Path-cne($Owner.RunDirectory+'.archive-local.json')){throw 'Entry stdout local acknowledgment path differs.'}
    # The original child's stdout pin is authority. Never adopt a later current
    # ack hash just because an independently readable file exists at that path.
    $ack=Open-RemovalDiagnosticEntryFile $published.Path 1048576 $Owner.ReadHandles
    Assert-RemovalDeliverySamePin $ack.Pin $published
    $value=Read-RemovalDiagnosticEntryJson $ack.Stream
    Assert-RemovalDeliveryFlags $value 'LOCAL_ARCHIVE_VERIFICATION_ONLY' $false
    if($value.Nonce-isnot[string]-or$value.Nonce-cnotmatch'\A[0-9a-f]{32}\z'){throw 'Original local acknowledgment nonce required.'}
    $archive=$output.Archive;Assert-RemovalDeliveryVersion $archive
    if($archive.Nonce-cne$value.Nonce-or$archive.LocalVerified-isnot[bool]-or-not$archive.LocalVerified-or
        $archive.Transferred-isnot[bool]-or$archive.Transferred-or$archive.NativePassed-isnot[bool]-or$archive.NativePassed-or
        $archive.Qualified-isnot[bool]-or$archive.Qualified){throw 'Entry stdout original export identity/flags differ.'}
    $controller=$output.ControllerResult;Assert-RemovalDeliveryObject $controller
    if($controller.Completed-isnot[bool]-or-not$controller.Completed-or$controller.NeedsObservation-isnot[bool]-or$controller.NeedsObservation-or
        $controller.Safe-isnot[bool]-or$controller.NativePassed-isnot[bool]-or$controller.NativePassed-or
        $controller.Qualified-isnot[bool]-or$controller.Qualified){throw 'Entry stdout does not describe a closed nonqualified controller.'}
    foreach($row in @(@{Key='ResultPin';Path=(Join-Path $Owner.RunDirectory 'controller-result.json');Cap=1048576},
        @{Key='ArchivePin';Path=($Owner.RunDirectory+'.zip');Cap=68157440},
        @{Key='InventoryPin';Path=($Owner.RunDirectory+'.zip.inventory.json');Cap=1048576})){
        $pin=$value.($row.Key);Assert-RemovalDeliveryPin $pin $row.Cap
        if($pin.Path-cne$row.Path){throw 'Original run result/archive/inventory path differs.'}
        Assert-RemovalDeliverySamePin $archive.($row.Key) $pin
        $actual=Open-RemovalDiagnosticEntryFile $row.Path $row.Cap $Owner.ReadHandles
        Assert-RemovalDeliverySamePin $actual.Pin $pin
        if($row.Key-ceq'ResultPin'){
            Assert-RemovalDeliverySamePin $controller.ReceiptPin $pin
            $saved=Read-RemovalDiagnosticEntryJson $actual.Stream;Assert-RemovalDeliveryVersion $saved
            if($saved.ControllerComplete-isnot[bool]-or-not$saved.ControllerComplete-or$saved.Nonce-cne$value.Nonce-or
                $saved.Safe-isnot[bool]-or$saved.Safe-ne$controller.Safe-or$saved.NativePassed-isnot[bool]-or$saved.NativePassed-or
                $saved.Qualified-isnot[bool]-or$saved.Qualified){throw 'Original saved controller result differs.'}
            Assert-RemovalDeliverySamePin $saved.Manifest $Owner.ManifestPin
        }
        if($row.Key-ceq'InventoryPin'){
            $inventory=Read-RemovalDiagnosticEntryJson $actual.Stream;Assert-RemovalDeliveryVersion $inventory
            if($inventory.Nonce-cne$value.Nonce){throw 'Original inventory nonce differs.'}
            Assert-RemovalDeliverySamePin $inventory.ResultPin $value.ResultPin
        }
    }
    # Rehash the same retained, writer-excluding handle, never reconstruct a pin
    # from mutable DTOs or reread external sources after the child failure.
    Assert-RemovalDeliverySamePin (Get-RemovalDiagnosticEntryStreamPin $published.Path $ack.Stream) $published
    return [pscustomobject]@{Path=$published.Path;Bytes=[long]$published.Bytes;Sha256=$published.Sha256.ToUpperInvariant()}
}

function New-RemovalDiagnosticEntrySink($Owner,$Channel){
    [Moiren.RemovalFiniteToolV1.Sink]::new($Owner.Files[$Channel.Index],$Owner.Budget)
}

function Start-RemovalDiagnosticEntryCopies($Owner){
    foreach($channel in $Owner.Channels){
        if($channel.DispatchState-cne'NotDispatched'){continue}
        try{
            # Source/sink construction cannot dispatch a pipe read. Publish each
            # original object before any later setup, and resume only absent ones.
            if($null-eq$channel.Source){
                $channel.Source=if($channel.Index-eq0){$Owner.Process.StandardOutput.BaseStream}else{$Owner.Process.StandardError.BaseStream}
            }
            if($null-eq$channel.Sink){
                $channel.Sink=New-RemovalDiagnosticEntrySink $Owner $channel
                $Owner.Sinks.Add($channel.Sink)
            }
            # A throwing CopyToAsync may already have read/dispatched work. Do
            # not infer "not started" from a missing returned Task, and never
            # call it again on that original stream after an ambiguous dispatch.
            $channel.DispatchState='Unknown'
            $channel.Copy=$channel.Source.CopyToAsync($channel.Sink,4096)
            if($null-eq$channel.Copy){throw 'Original pipe copy returned no task.'}
            $channel.DispatchState='Dispatched'
            $Owner.Copies.Add($channel.Copy)
        }catch{
            Add-RemovalDiagnosticEntryError $Owner ('Original entry channel '+$channel.Index+' setup/dispatch failed; retain exact objects.')
        }
    }
}

function Wait-RemovalDiagnosticEntryOwner($Owner){
    # A sticky deadline/observer exception is not permission to abandon the
    # original process or its two original CopyToAsync tasks. Completion may
    # remain pending indefinitely if the original owner cannot be proven closed.
    while($true){
        if($Owner.Clock.ElapsedMilliseconds-ge600000){$Owner.TimedOut=$true;Add-RemovalDiagnosticEntryError $Owner 'Entry deadline exceeded; original owner retained.'}
        try{
            if(-not$Owner.Started){
                # If Start threw after associating this same Process, the exact
                # retained handle can still establish dispatch; never Start again.
                $Owner.Handle=$Owner.Process.Handle
                if($Owner.Handle-ne[IntPtr]::Zero){$Owner.Started=$true}
            }
            if($Owner.Started){Start-RemovalDiagnosticEntryCopies $Owner}
            if($null-eq$Owner.Pid-or$null-eq$Owner.CreationUtcTicks){
                $Owner.Handle=$Owner.Process.Handle;$Owner.Pid=$Owner.Process.Id
                $Owner.CreationUtcTicks=$Owner.Process.StartTime.ToUniversalTime().Ticks
            }
            if(-not$Owner.ExitedConfirmed-and$Owner.Process.WaitForExit(0)){
                $Owner.ExitCode=$Owner.Process.ExitCode;$Owner.ExitedConfirmed=$true
            }
            if($Owner.ExitedConfirmed-and$Owner.Channels.Count-eq2-and
                @($Owner.Channels|Where-Object{$_.DispatchState-cne'Dispatched'-or$null-eq$_.Copy-or-not$_.Copy.IsCompleted}).Count-eq0){
                foreach($channel in $Owner.Channels){try{$null=$channel.Copy.GetAwaiter().GetResult()}catch{Add-RemovalDiagnosticEntryError $Owner 'Original entry pipe drain failed.'}}
                break
            }
        }catch{Add-RemovalDiagnosticEntryError $Owner 'Original entry observation failed; retaining process and pipes.'}
        Start-Sleep -Milliseconds 20
    }
    foreach($file in $Owner.Files){try{$file.Flush($true)}catch{Add-RemovalDiagnosticEntryError $Owner 'Entry output durable flush failed.'}}
    $sinkFailed=@($Owner.Sinks|Where-Object{$_.Failed}).Count-gt0
    if($Owner.Budget.Exceeded){Add-RemovalDiagnosticEntryError $Owner 'Shared entry stdout/stderr 1 MiB cap exceeded.'}
    if($sinkFailed){Add-RemovalDiagnosticEntryError $Owner 'Entry output sink failed; original child drained.'}
    $Owner.OutputStable=(-not$Owner.Budget.Exceeded-and-not$sinkFailed-and
        @($Owner.Copies|Where-Object{$_.IsFaulted-or$_.IsCanceled}).Count-eq0-and
        -not$Owner.Errors.Contains('Entry output durable flush failed.'))
    if($Owner.Clock.ElapsedMilliseconds-ge600000){$Owner.TimedOut=$true;Add-RemovalDiagnosticEntryError $Owner 'Entry deadline exceeded; original owner retained.'}
}

function Invoke-RemovalDiagnosticEntry {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$RunDirectory,
        [Parameter(Mandatory=$true)][string]$ManifestPath,
        [Parameter(Mandatory=$true)][ValidatePattern('\A[A-Fa-f0-9]{64}\z')][string]$ManifestSha256,
        [Parameter(Mandatory=$true)][string]$SourceDirectory,
        [Parameter(Mandatory=$true)]$PowerShellPin,[switch]$UserRebootConfirmed,
        [ValidateRange(0,65535)][int]$WorkerSessionId=0)
    $ErrorActionPreference='Stop'
    if(-not$UserRebootConfirmed){throw 'User reboot confirmation required; entry never reboots.'}
    Assert-RemovalDiagnosticEntryTarget $RunDirectory
    foreach($path in @($RunDirectory,$ManifestPath,$SourceDirectory)){Assert-FiniteToolPath $path $false}
    if(-not[IO.Directory]::Exists([IO.Path]::GetDirectoryName($RunDirectory))-or-not[IO.Directory]::Exists($SourceDirectory)){
        throw 'Existing run parent and production source directory required.'
    }
    Assert-RemovalDeliveryPin $PowerShellPin 268435456
    if([IO.Path]::GetFileName($PowerShellPin.Path)-cne'powershell.exe'){throw 'Exact pinned Windows PowerShell executable required.'}
    Initialize-FiniteToolSink
    $handles=New-Object Collections.Generic.List[object]
    $files=New-Object Collections.Generic.List[object]
    $owner=$null
    try{
        $manifest=Open-RemovalDiagnosticEntryFile $ManifestPath 1048576 $handles
        if($manifest.Pin.Sha256-ine$ManifestSha256){throw 'Independent diagnostic manifest hash differs.'}
        $shell=Open-RemovalDiagnosticEntryFile $PowerShellPin.Path 268435456 $handles
        Assert-RemovalDeliverySamePin $shell.Pin $PowerShellPin
        $self=Open-RemovalDiagnosticEntryFile (Join-Path $SourceDirectory 'entry-launch.ps1') 1048576 $handles
        if($self.Pin.Path-cne$script:RemovalDiagnosticEntrySourcePath){throw 'Imported wrapper is not the exact production source entry.'}
        $child=Open-RemovalDiagnosticEntryFile (Get-RemovalDiagnosticEntryScript $SourceDirectory) 1048576 $handles
        $paths=@(($RunDirectory+'.entry-stdout.log'),($RunDirectory+'.entry-stderr.log'),($RunDirectory+'.entry-intent.json'),($RunDirectory+'.entry-terminal.json'))
        foreach($path in $paths){Assert-FiniteToolPath $path $false;if(Test-Path -LiteralPath $path){throw 'Entry attempt output exists; no overwrite or relaunch.'}}
        $argv=@('-NoLogo','-NoProfile','-NonInteractive','-File',$child.Pin.Path,'-RunDirectory',$RunDirectory,
            '-ManifestPath',$ManifestPath,'-ManifestSha256',$ManifestSha256,'-UserRebootConfirmed','-WorkerSessionId',[string]$WorkerSessionId)
        $line=(@($argv|ForEach-Object{Quote-FiniteToolArgument $_})-join' ')
        if($line.Length+$PowerShellPin.Path.Length+3-gt32766){throw 'Entry command line exceeds Windows bound.'}
        $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=$PowerShellPin.Path;$start.Arguments=$line
        $start.WorkingDirectory=$SourceDirectory;$start.UseShellExecute=$false;$start.CreateNoWindow=$true
        $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
        $owner=[pscustomobject]@{RunDirectory=$RunDirectory;ManifestPin=$manifest.Pin;Process=[Diagnostics.Process]::new()
            Handle=[IntPtr]::Zero;Pid=$null;CreationUtcTicks=$null;LaunchAttempted=$false;Started=$false;NoDispatchConfirmed=$false
            ExitedConfirmed=$false;ExitCode=$null;TimedOut=$false;OutputStable=$false;Clock=$null;Closed=$false
            Channels=@([pscustomobject]@{Index=0;Source=$null;Sink=$null;Copy=$null;DispatchState='NotDispatched'},
                [pscustomobject]@{Index=1;Source=$null;Sink=$null;Copy=$null;DispatchState='NotDispatched'})
            Copies=(New-Object Collections.Generic.List[object]);Sinks=(New-Object Collections.Generic.List[object]);Files=$files
            Budget=[Moiren.RemovalFiniteToolV1.Budget]::new(1048576);ReadHandles=$handles
            Errors=(New-Object Collections.Generic.List[string])}
        $owner.Process.StartInfo=$start
        $null=Write-RemovalDiagnosticEntryJson $paths[2] ([ordered]@{SchemaVersion=1;Meaning='ORIGINAL_DIAGNOSTIC_ENTRY_INTENT'
            Target='moiren-lan';RunDirectory=$RunDirectory;ManifestPin=$manifest.Pin;EntrySourcePin=$self.Pin;ChildSourcePin=$child.Pin
            PowerShellPin=$shell.Pin;Arguments=@($argv);WorkerSessionId=$WorkerSessionId;DeadlineSeconds=600
            NativePassed=$false;Qualified=$false})
        foreach($path in $paths[0..1]){$files.Add([IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read))}
        # Publish the exact Process and writable files before Process.Start; even
        # a throwing/ambiguous launch keeps this owner rather than starting again.
        $script:RemovalDiagnosticEntryOwners.Add($owner)
        $owner.Clock=[Diagnostics.Stopwatch]::StartNew();$owner.LaunchAttempted=$true
        try{
            $owner.Started=$owner.Process.Start()
            if(-not$owner.Started){
                # A normal false return on this newly created, non-shell Process
                # confirms no child was started. A thrown Start is NOT this case.
                $owner.NoDispatchConfirmed=$true;throw 'Entry Start returned no child.'
            }
            Start-RemovalDiagnosticEntryCopies $owner
            $owner.Handle=$owner.Process.Handle;$owner.Pid=$owner.Process.Id
            $owner.CreationUtcTicks=$owner.Process.StartTime.ToUniversalTime().Ticks
            if($owner.Handle-eq[IntPtr]::Zero-or$owner.Pid-le0-or$owner.CreationUtcTicks-le0){throw 'Original entry process identity unavailable.'}
        }catch{Add-RemovalDiagnosticEntryError $owner 'Entry launch/identity/pipe setup failed; original owner retained.'}
        if($owner.NoDispatchConfirmed){throw 'Entry Start confirmed no child; preserve intent without inventing a terminal process.'}
        Wait-RemovalDiagnosticEntryOwner $owner
        if($owner.ExitCode-ne0-and$owner.ExitCode-ne1){Add-RemovalDiagnosticEntryError $owner 'Unexpected original entry exit code; no delivery.'}
        $ack=$null
        if($owner.OutputStable-and-not$owner.TimedOut-and$owner.Errors.Count-eq0){
            try{$ack=Assert-RemovalDiagnosticEntryLocalAck $owner}
            catch{Add-RemovalDiagnosticEntryError $owner ('Original entry local acknowledgment refused: '+$_.Exception.Message)}
        }
        $stdout=Get-RemovalDiagnosticEntryStreamPin $paths[0] $files[0]
        $stderr=Get-RemovalDiagnosticEntryStreamPin $paths[1] $files[1]
        $receipt=[ordered]@{SchemaVersion=1;Meaning='ORIGINAL_DIAGNOSTIC_ENTRY_TERMINAL';Target='moiren-lan'
            RunDirectory=$RunDirectory;ManifestPin=$manifest.Pin;EntrySourcePin=$self.Pin;ChildSourcePin=$child.Pin
            Process=[ordered]@{Pid=$owner.Pid;CreationUtcTicks=$owner.CreationUtcTicks;ExitedConfirmed=[bool]$owner.ExitedConfirmed}
            ExitCode=$owner.ExitCode;TimedOut=[bool]$owner.TimedOut;OutputStable=[bool]$owner.OutputStable;Errors=@($owner.Errors.ToArray())
            StdoutPin=$stdout;StderrPin=$stderr;LocalAcknowledgementPin=$ack;NativePassed=$false;Qualified=$false}
        $pin=Write-RemovalDiagnosticEntryJson $paths[3] $receipt
        # Only natural exit + both original drains + persisted terminal evidence
        # release the original process, files, ack read handles and source pins.
        foreach($file in $files){$file.Dispose()}
        $owner.Process.Dispose();$owner.Closed=$true
        $null=$script:RemovalDiagnosticEntryOwners.Remove($owner)
        return $pin
    }finally{
        if($null-eq$owner-or-not$owner.LaunchAttempted-or$owner.NoDispatchConfirmed-or$owner.Closed){
            foreach($file in $files){$file.Dispose()}
            foreach($handle in $handles){$handle.Dispose()}
            if($null-ne$owner-and(-not$owner.LaunchAttempted-or$owner.NoDispatchConfirmed)){
                $owner.Process.Dispose();$owner.Closed=$true;$null=$script:RemovalDiagnosticEntryOwners.Remove($owner)
            }
        }
    }
}
