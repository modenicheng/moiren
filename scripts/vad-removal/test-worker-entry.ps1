#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'finite-tool.ps1')

# A valid stage must stop at the real host guard, never at a mocked launch gate.
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){
    throw 'Run worker-entry rejection tests only on a non-target host.'
}
$checks=New-Object Collections.Generic.List[object]
$retained=New-Object Collections.Generic.List[object]
$root=Join-Path $PSScriptRoot ('task-fixtures\worker-entry-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$hostExecutable=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
$libraryNames=@('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1')

function Check([bool]$Value,[string]$Why){if(-not $Value){throw $Why}}
function Case([string]$Name,[scriptblock]$Action){
    try {&$Action;$checks.Add(@{name=$Name;passed=$true})}
    catch {$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}
}
function Write-Utf8([string]$Path,[string]$Text){
    [IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($false))
}
function Save-Configuration($Fixture){
    Write-Utf8 (Join-Path $Fixture.RunDirectory 'run-spec.json') (ConvertTo-Json -InputObject $Fixture.Config -Depth 8 -Compress)
    $Fixture.ConfigHash=(Get-FileHash -LiteralPath (Join-Path $Fixture.RunDirectory 'run-spec.json') -Algorithm SHA256).Hash
}
function Fixture {
    $run=Join-Path $root ([Guid]::NewGuid().ToString('N'))
    $null=[IO.Directory]::CreateDirectory($run)
    foreach($name in ($libraryNames+@('worker.ps1'))){
        [IO.File]::WriteAllBytes((Join-Path $run $name),[IO.File]::ReadAllBytes((Join-Path $PSScriptRoot $name)))
    }
    $pins=@(foreach($name in $libraryNames){
        @{Name=$name;Sha256=(Get-FileHash -LiteralPath (Join-Path $run $name) -Algorithm SHA256).Hash}
    })
    $self=[Diagnostics.Process]::GetCurrentProcess()
    $config=@{SchemaVersion=1;Nonce=[Guid]::NewGuid().ToString('N');RunDirectory=$run;SessionId=2
        Sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;ControllerPid=$PID
        ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks;Executable=$hostExecutable
        ExpectedSha256=(Get-FileHash -LiteralPath $hostExecutable -Algorithm SHA256).Hash
        ProbeArguments=@('live-vad','--seconds','8');Libraries=$pins}
    $f=[pscustomobject]@{RunDirectory=$run;Config=$config;ConfigHash=$null}
    Save-Configuration $f
    return $f
}
function Invoke-Entry($Fixture,[string]$ConfigHash=$Fixture.ConfigHash){
    $entry=Join-Path $Fixture.RunDirectory 'worker.ps1'
    $argv=@('-NoLogo','-NoProfile','-NonInteractive','-WindowStyle','Hidden','-File',$entry,
        '-TaskPath',('\Moiren-Removal613-'+$Fixture.Config.Nonce),'-Nonce',$Fixture.Config.Nonce,
        '-ExpectedSessionId','2','-RunDirectory',$Fixture.RunDirectory,
        '-WorkerSha256',(Get-FileHash -LiteralPath $entry -Algorithm SHA256).Hash,'-RunSpecSha256',$ConfigHash)
    $p=[Diagnostics.Process]::new()
    $p.StartInfo=[Diagnostics.ProcessStartInfo]::new()
    $p.StartInfo.FileName=$hostExecutable
    $p.StartInfo.Arguments=($argv|ForEach-Object{Quote-FiniteToolArgument $_}) -join ' '
    $p.StartInfo.UseShellExecute=$false;$p.StartInfo.CreateNoWindow=$true
    $p.StartInfo.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $p.StartInfo.RedirectStandardOutput=$true;$p.StartInfo.RedirectStandardError=$true
    $p.StartInfo.EnvironmentVariables['PSModulePath']=Join-Path $PSHOME 'Modules'
    try {
        Check ($p.Start()) 'Production worker entry did not start.'
        $stdout=$p.StandardOutput.ReadToEndAsync();$stderr=$p.StandardError.ReadToEndAsync()
        if(-not $p.WaitForExit(15000)){
            $retained.Add($p)
            throw 'Harmless entry did not exit; original process retained, no kill/relaunch.'
        }
        $result=[pscustomobject]@{ExitCode=$p.ExitCode;Text=($stdout.GetAwaiter().GetResult()+$stderr.GetAwaiter().GetResult())}
        Check ($result.ExitCode -eq 1) ('Entry did not refuse: '+$result.Text)
        foreach($name in @('worker-start.json','worker-probe-start.json','probe-ready.json','probe-admit.json','worker-result.json')){
            Check (-not [IO.File]::Exists((Join-Path $Fixture.RunDirectory $name))) ('Entry crossed native boundary: '+$name)
        }
        return $result
    } finally {
        if(-not $retained.Contains($p)){$p.Dispose()}
    }
}
function Expect-EntryError($Fixture,[string]$Message,[string]$ConfigHash=$Fixture.ConfigHash){
    $result=Invoke-Entry $Fixture $ConfigHash
    Check ($result.Text.Contains($Message)) ('Wrong refusal; expected '+$Message+'; got '+$result.Text)
}
function Pin-Library($Fixture,[string]$Name){
    foreach($pin in $Fixture.Config.Libraries){
        if($pin.Name -ceq $Name){$pin.Sha256=(Get-FileHash -LiteralPath (Join-Path $Fixture.RunDirectory $Name) -Algorithm SHA256).Hash}
    }
    Save-Configuration $Fixture
}

Case 'actual-entry-valid-stage-stops-at-unmodified-wrong-host-guard' {
    $f=Fixture
    Expect-EntryError $f 'Wrong scheduler target host/SID.'
}
Case 'actual-entry-rejects-configuration-pin-mismatch' {
    $f=Fixture
    Expect-EntryError $f 'Pinned run configuration differs.' ('0'*64)
}
Case 'actual-entry-caps-configuration-before-hashing' {
    $f=Fixture
    [IO.File]::WriteAllBytes((Join-Path $f.RunDirectory 'run-spec.json'),(New-Object byte[] 8193))
    Expect-EntryError $f 'Run configuration length invalid.' ('0'*64)
}
Case 'actual-entry-does-not-auto-detect-utf16-in-pinned-utf8-json' {
    $f=Fixture
    $text=ConvertTo-Json -InputObject $f.Config -Depth 8 -Compress
    $utf16=[Text.Encoding]::Unicode.GetPreamble()+[Text.Encoding]::Unicode.GetBytes($text)
    [IO.File]::WriteAllBytes((Join-Path $f.RunDirectory 'run-spec.json'),$utf16)
    $f.ConfigHash=(Get-FileHash -LiteralPath (Join-Path $f.RunDirectory 'run-spec.json') -Algorithm SHA256).Hash
    Expect-EntryError $f 'Run configuration UTF-8 invalid.'
}
Case 'actual-entry-rejects-pinned-malformed-json' {
    $f=Fixture
    Write-Utf8 (Join-Path $f.RunDirectory 'run-spec.json') '{"'
    $f.ConfigHash=(Get-FileHash -LiteralPath (Join-Path $f.RunDirectory 'run-spec.json') -Algorithm SHA256).Hash
    Expect-EntryError $f 'Run configuration JSON invalid.'
}
Case 'actual-entry-rejects-wrong-run-identity' {
    $f=Fixture;$f.Config.Nonce='0'*32;Save-Configuration $f
    $f.Config.Nonce='1'*32
    Expect-EntryError $f 'Wrong exact run configuration.'
}
Case 'actual-entry-rejects-reordered-source-manifest' {
    $f=Fixture;$f.Config.Libraries=@($f.Config.Libraries[1],$f.Config.Libraries[0],$f.Config.Libraries[2],$f.Config.Libraries[3])
    Save-Configuration $f
    Expect-EntryError $f 'Wrong ordered source manifest.'
}
Case 'actual-entry-rejects-library-pin-mismatch-before-loading' {
    $f=Fixture
    Write-Utf8 (Join-Path $f.RunDirectory 'worker-boundary.ps1') "throw 'UNVERIFIED-LIBRARY-EXECUTED'"
    Expect-EntryError $f 'Worker library pin differs.'
}
Case 'actual-entry-caps-source-before-hashing-or-loading' {
    $f=Fixture
    [IO.File]::WriteAllBytes((Join-Path $f.RunDirectory 'worker-boundary.ps1'),(New-Object byte[] 262145))
    Expect-EntryError $f 'Worker library length invalid.'
}
Case 'actual-entry-refuses-reparse-ancestor-before-any-library-load' {
    $f=Fixture
    Write-Utf8 (Join-Path $f.RunDirectory 'worker-boundary.ps1') "throw 'REPARSE-LIBRARY-EXECUTED'"
    Pin-Library $f 'worker-boundary.ps1'
    $alias=Join-Path $root ('junction-'+[Guid]::NewGuid().ToString('N'))
    $null=New-Item -ItemType Junction -Path $alias -Target $f.RunDirectory
    $f.RunDirectory=$alias;$f.Config.RunDirectory=$alias;Save-Configuration $f
    Expect-EntryError $f 'Reparse worker source path refused.'
}
Case 'actual-entry-holds-every-source-and-config-lock-through-nested-dot-source' {
    $f=Fixture
    $lockFixture=@'
. (Join-Path $PSScriptRoot 'probe-boundary.ps1')
foreach($name in @('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1','worker.ps1','run-spec.json')){
    $path=Join-Path $PSScriptRoot $name
    try {
        $writer=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::ReadWrite)
        $writer.Dispose()
        throw ('Source write lock missing: '+$name)
    } catch [IO.IOException] {}
    try {[IO.File]::Delete($path);throw ('Source delete lock missing: '+$name)} catch [IO.IOException] {}
    try {[IO.File]::Move($path,($path+'.moved'));throw ('Source rename lock missing: '+$name)} catch [IO.IOException] {}
}
throw 'All pinned source locks verified; fixture stops before worker invocation.'
'@
    Write-Utf8 (Join-Path $f.RunDirectory 'worker-boundary.ps1') $lockFixture
    Pin-Library $f 'worker-boundary.ps1'
    Expect-EntryError $f 'All pinned source locks verified; fixture stops before worker invocation.'
    foreach($name in ($libraryNames+@('worker.ps1','run-spec.json'))){
        $path=Join-Path $f.RunDirectory $name
        $writer=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::ReadWrite)
        $writer.Dispose()
        [IO.File]::Move($path,($path+'.unlocked'));[IO.File]::Move(($path+'.unlocked'),$path)
    }
}
Case 'actual-entry-catch-observes-retained-owner-through-error-before-exit' {
    $f=Fixture
    $ownerFixture=@'
$script:RemovalWorkerProbeOwner=[pscustomobject]@{LaunchAttempted=$true;Closed=$false;Observations=0}
function Observe-RemovalWorkerProbe($Owner,[int]$ObservationMilliseconds){
    if(-not [object]::ReferenceEquals($Owner,$script:RemovalWorkerProbeOwner)){throw 'Original owner replaced.'}
    $Owner.Observations++
    if($Owner.Observations -eq 1){throw 'Transient observation error.'}
    [Console]::Out.WriteLine('Original owner observed '+$Owner.Observations)
    return @{NeedsObservation=($Owner.Observations -lt 3)}
}
function Close-RemovalWorkerProbeOwner($Owner){
    if($Owner.Observations -ne 3){throw 'Unresolved owner closed.'}
    $Owner.Closed=$true
    [Console]::Out.WriteLine('Original owner closed after terminal observation.')
}
function Add-FiniteToolError($Owner,[string]$Message){}
throw 'Unexpected entry failure with retained owner.'
'@
    Write-Utf8 (Join-Path $f.RunDirectory 'worker-boundary.ps1') $ownerFixture
    Pin-Library $f 'worker-boundary.ps1'
    $result=Invoke-Entry $f
    Check ($result.Text.Contains('Unexpected entry failure with retained owner.')) 'Unexpected failure was hidden.'
    Check ($result.Text.Contains('Original owner observed 2') -and $result.Text.Contains('Original owner observed 3') -and
        $result.Text.Contains('Original owner closed after terminal observation.')) 'Entry catch abandoned original owner after an observation error.'
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not $_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' ENTRY=production HASH_PARSE=real SCHEDULER=disabled NATIVE_EXECUTED=false')
if($failed){exit 1}
