#requires -Version 5.1
# Shared decode fixtures only: dot-source trace-decode.ps1 first and provide
# Check/Reject assertion helpers. All ETW/process-metadata boundaries are mocked;
# tool fixtures launch only harmless PowerShell and the existing offline reader.
# Dot-sourcing keeps $script:fixture selected for boundary closures and exposes
# $script:realOwners so the caller retains/observes every finite child owner.
. (Join-Path $PSScriptRoot 'trace-session.ps1')
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
. (Join-Path $PSScriptRoot 'collector-boundary.ps1')
function Assert-RemovalTaskHost {}
function Open-RemovalWorkerProcess([int]$WorkerPid){return $script:fixture.WorkerProcess}
function Read-RemovalWorkerProcessMetadata($Process){return $script:fixture.WorkerMetadata}
function Open-RemovalProbeProcess([int]$ProbePid){return $script:fixture.ProbeProcess}
function Read-RemovalProbeMetadata($Process){return $script:fixture.ProbeMetadata}
Initialize-RemovalTraceNative
$script:fixtureRoot=Join-Path $PSScriptRoot ('logs\decode-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($script:fixtureRoot)
$script:ps=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$script:psPin=@{Path=$script:ps;Bytes=(Get-Item $script:ps).Length;Sha256=(Get-FileHash $script:ps -Algorithm SHA256).Hash}
$script:python=(Get-Command python.exe -ErrorAction Stop).Source
$script:realOwners=New-Object Collections.Generic.List[object]
function Write-Text([string]$Path,[string]$Text){[IO.File]::WriteAllText($Path,$Text,[Text.UTF8Encoding]::new($false))}
function Pin([string]$Path){@{Path=$Path;Bytes=(Get-Item -LiteralPath $Path).Length;Sha256=(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash}}
function Write-Buffer([IntPtr]$Pointer,[UInt64]$Handle,[UInt32]$Loss){
 $v=[Runtime.InteropServices.Marshal]::PtrToStructure($Pointer,[type][Moiren.RemovalTraceQueryV1.Properties])
 $w=$v.Wnode;$w.Guid=$script:fixture.State.Guid;$w.HistoricalContext=$Handle;$w.ClientContext=1;$v.Wnode=$w
 $v.LogFileMode=1;$v.MaximumFileSize=16;$v.EventsLost=$Loss
 [Runtime.InteropServices.Marshal]::StructureToPtr($v,$Pointer,$false)
 foreach($entry in @(@{Offset=$v.LoggerNameOffset;Text=$script:fixture.State.Session},@{Offset=$v.LogFileNameOffset;Text=$script:fixture.State.Etl})){
  $bytes=[Text.Encoding]::Unicode.GetBytes($entry.Text+[char]0)
  [Runtime.InteropServices.Marshal]::Copy($bytes,0,[IntPtr]::Add($Pointer,[int]$entry.Offset),$bytes.Length)
 }
}
function Enumerate-Provider([IntPtr]$Pointers,[UInt32]$Capacity,[IntPtr]$Returned){
 $row=New-Object Moiren.RemovalTraceQueryV1.GuidProperties
 $row.Guid=[Guid]'7dc75ba9-8274-4eae-a42d-b4c74e0209d2';$row.LoggerId=17
 $row.IsEnable=[byte]$script:fixture.State.Enabled;$row.EnableLevel=4;$row.EnableFlags=15
 [Runtime.InteropServices.Marshal]::StructureToPtr($row,[Runtime.InteropServices.Marshal]::ReadIntPtr($Pointers),$false)
 [Runtime.InteropServices.Marshal]::WriteInt32($Returned,1);return [UInt32]0
}
function New-Fixture([UInt32]$StopStatus=0,[switch]$LeaveTraceRunning){
 $root=Join-Path $script:fixtureRoot ([Guid]::NewGuid().ToString('N'));$null=[IO.Directory]::CreateDirectory($root)
 $tmf=Join-Path $root 'tmf';$null=[IO.Directory]::CreateDirectory($tmf)
 $tracefmt=Join-Path $root 'tracefmt.exe';[IO.File]::WriteAllBytes($tracefmt,[byte[]](1,2,3))
 $etl=Join-Path $root 'driver.etl';[IO.File]::WriteAllBytes($etl,[byte[]](5,6,7,8))
 $probe=Join-Path $root 'e2e_latency_probe.exe';[IO.File]::WriteAllBytes($probe,[byte[]](9,10,11))
 $package=Join-Path $root 'package-manifest.json'
 Write-Text $package ('{"SchemaVersion":1,"DriverVersion":"3.1.1.613","Revision":"'+('a'*40)+'","PdbSha256":"'+('b'*64)+'","Files":[{"Name":"MoirenVad.sys","Sha256":"'+('c'*64)+'"}]}')
 $tmfFile=Join-Path $tmf 'ebd9d202-6309-30db-45cd-598f0215081c.tmf'
 Write-Text $tmfFile 'synthetic TMF pin; tests never run tracefmt'
 $run=Join-Path $root 'managed-run';$artifacts=Join-Path $run 'artifacts'
 $report=Join-Path $artifacts 'report.json'
 $manifest=Join-Path $root 'probe-build.json'
 $probePin=Pin $probe
 Write-Text $manifest (@{Path=$probe;Bytes=$probePin.Bytes;Sha256=$probePin.Sha256;Source=('d'*40)}|ConvertTo-Json)
 $reader=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\vad-removal-stages.py'))
 $script:fixture=@{Root=$root;Mode='normal';Calls=New-Object Collections.Generic.List[object];Owners=New-Object Collections.Generic.List[object]
  State=@{Guid=[Guid]::Empty;Enabled=0;Session='MoirenRemoval613decode';Etl=$etl;StopStatus=$StopStatus;Loss=[UInt32]3}}
 $managed=New-RemovalManagedSession -RunDirectory $run -Executable $probe -ExpectedSha256 $probePin.Sha256 `
  -ProbeArguments @('live-vad','--artifact-dir',$artifacts,'--seconds','8') -SessionId 2 `
  -Sid 'S-1-5-21-3080296863-1242338503-1346362208-1001' -Nonce ([Guid]::NewGuid().ToString('N'))
 $null=[IO.Directory]::CreateDirectory($artifacts)
 Write-Text $report '{"native":{"error":"VAD cleanup did not reach Removed","vad_playback":{"worker_joined":true,"native_stop_succeeded":true,"failure":null},"engine_cleanup":{"host":"Closed","fault":"None","cleanup_error":"None"},"vad_cleanup":{"diagnostic_provenance":"Fresh","error":null,"snapshot":{"stamp":[21,99,21,6,8],"state":"Removing"}}}}'
 $native=@{
  Start={param([IntPtr]$Handle,[string]$Name,[IntPtr]$Properties)$v=[Runtime.InteropServices.Marshal]::PtrToStructure($Properties,[type][Moiren.RemovalTraceQueryV1.Properties]);$script:fixture.State.Guid=$v.Wnode.Guid;[Runtime.InteropServices.Marshal]::WriteInt64($Handle,701);return [UInt32]0}
  Enumerate={param([IntPtr]$Pointers,[UInt32]$Capacity,[IntPtr]$Returned)Enumerate-Provider $Pointers $Capacity $Returned}
  Enable={param($a,$b,$c,$d,$e,$f,$g)$script:fixture.State.Enabled=1;return [UInt32]0}
  Query={param([UInt64]$Handle,[IntPtr]$Properties)Write-Buffer $Properties 701 0;return [UInt32]0}
  Stop={param([UInt64]$Handle,[IntPtr]$Properties)Write-Buffer $Properties 701 $script:fixture.State.Loss;return [UInt32]$script:fixture.State.StopStatus}
 }
 $query=@{
  Query={param([UInt64]$Handle,[IntPtr]$Properties)Write-Buffer $Properties 701 0;return [UInt32]0}
  Enumerate={param([IntPtr]$Pointers,[UInt32]$Capacity,[IntPtr]$Returned)Enumerate-Provider $Pointers $Capacity $Returned}
 }
 $session=New-RemovalTraceSession -Session $script:fixture.State.Session -EtlPath $etl -NativeBoundary $native -QueryBoundary $query
 Start-RemovalTraceSession $session
 # Composition callers retain the same selected fixture and perform STOP later.
 if(-not$LeaveTraceRunning){
  if($StopStatus-eq0){Stop-RemovalTraceSession $session}else{Reject {Stop-RemovalTraceSession $session}}
 }
 $pins=@{SchemaVersion=1;DriverPackage=(Pin $package);Etl=(Pin $etl);Tracefmt=(Pin $tracefmt)
  Python=(Pin $script:python);Reader=(Pin $reader);ProbeReport=(Pin $report)
  Probe=@{File=(Pin $probe);BuildManifest=(Pin $manifest);SourceRevision=('d'*40)}
  Symbols=@{DriverRevision=('a'*40);PdbSha256=('b'*64);Directory=$tmf;Files=@((Pin $tmfFile))}}
 $managed.TraceSession=$session;$managed.TraceReadback=$session.Readback
 Initialize-ManagedFixture $managed
 $script:fixture.Session=$session;$script:fixture.Pins=$pins;$script:fixture.ManagedSession=$managed
 return $script:fixture
}
function Initialize-ManagedFixture($Managed){
 $s=$Managed.TaskOwner.Spec
 $task=[pscustomobject]@{Path=$s.Path;Name=$s.Name;Enabled=$true;State=3;LastTaskResult=1;LastRunTime=[DateTime]::Now;Xml='<mock exact scheduler/>'
  Definition=[pscustomobject]@{Principal=[pscustomobject]@{UserId=$s.Sid;LogonType=3;RunLevel=0};Triggers=[pscustomobject]@{Count=0}
   Actions=[pscustomobject]@{Count=1;Action=[pscustomobject]@{Type=0;Path=$s.Executable;Arguments=$s.Arguments;WorkingDirectory=$s.RunDirectory}}
   Settings=[pscustomobject]@{Enabled=$true;AllowDemandStart=$true;MultipleInstances=2;RestartCount=0;AllowHardTerminate=$false;ExecutionTimeLimit='PT0S';StopIfGoingOnBatteries=$false;DisallowStartIfOnBatteries=$false}}}
 $task.Definition.Actions|Add-Member ScriptMethod Item {param($i)return $this.Action}
 $task|Add-Member ScriptMethod GetInstances {param($flags)return [pscustomobject]@{Count=0}}
 $o=New-RemovalTaskOwner -Spec $s -RegisteredTask $task
 $o.DispatchAttempted=$true;$o.DispatchUtc=[DateTimeOffset]::UtcNow.AddSeconds(-1).ToString('o');$o.InstanceGuid='{b4c9f882-3201-4e53-b926-8dd40f3b46c9}'
 $created=[DateTime]::UtcNow
 $r=@{Nonce=$s.Nonce;TaskPath=$s.Path;Host='LAPTOP-NP49RG6H';Sid=$s.Sid;SessionId=$s.SessionId
  IsAdministrator=$false;Pid=5678;CreationUtcTicks=$created.Ticks;WorkerSha256=$s.WorkerSha256
  ProbeExitKnown=$true;ProbeExitCode=1;ProbePid=9876;ProbeCreationUtcTicks=$created.Ticks
  OutputStable=$true;TimedOut=$false;LogLimitExceeded=$false;Errors=@();Owners=@();WorkerExitCode=1}
 $wp=[pscustomobject]@{Id=[int]5678;Handle=[IntPtr]41;StartTime=$created.ToLocalTime();SessionId=$s.SessionId
  MainModule=[pscustomobject]@{FileName=$s.Executable};HasExited=$false;ExitCode=1}
 $wp|Add-Member ScriptMethod Refresh {}
 $script:fixture.WorkerProcess=$wp
 $script:fixture.WorkerMetadata=@{Pid=5678;Sid=$s.Sid;IsAdministrator=$false;IsElevated=$false;Executable=$s.Executable;CommandLine=('"'+$s.Executable+'" '+$s.Arguments)}
 Attach-RemovalWorkerProcess $o $r
 $spec=New-RemovalProbeSpec -Executable $Managed.Config.Executable -ExpectedSha256 $Managed.Config.ExpectedSha256 `
  -RunDirectory $s.RunDirectory -Nonce $s.Nonce -SessionId $s.SessionId -Sid $s.Sid `
  -ControllerPid $Managed.ControllerPid -ControllerCreationUtcTicks $Managed.ControllerCreationUtcTicks `
  -WorkerPid $o.WorkerPid -WorkerCreationUtcTicks $o.WorkerCreationUtcTicks -ProbeArguments $Managed.Config.ProbeArguments
 $ready=@{schema_version=1;nonce=$spec.Nonce;probe_pid=9876;probe_creation_utc_ticks=$created.Ticks
  controller_pid=$spec.ControllerPid;controller_creation_utc_ticks=$spec.ControllerCreationUtcTicks
  worker_pid=$spec.WorkerPid;worker_creation_utc_ticks=$spec.WorkerCreationUtcTicks;session_id=$spec.SessionId}
 $pp=[pscustomobject]@{Id=[int]9876;Handle=[IntPtr]53;StartTime=$created.ToLocalTime();SessionId=$s.SessionId
  MainModule=[pscustomobject]@{FileName=$spec.Executable};HasExited=$false;ExitCode=1}
 $pp|Add-Member ScriptMethod Refresh {}
 $pp|Add-Member ScriptMethod WaitForExit {param($ms)return $this.HasExited}
 $script:fixture.ProbeProcess=$pp
 $script:fixture.ProbeMetadata=@{Pid=9876;ParentPid=$o.WorkerPid;Sid=$s.Sid;IsAdministrator=$false;IsElevated=$false
  Executable=$spec.Executable;CommandLine=('"'+$spec.Executable+'" '+$spec.CommandLine)}
 $po=New-RemovalProbeOwner $spec;Attach-RemovalProbeProcess $po $ready;Admit-RemovalProbe $po
 $wp.HasExited=$true;$pp.HasExited=$true
 $Managed.TaskOwner=$o;$Managed.ProbeOwner=$po;$Managed.LastResult=$r
}
function Write-DecodedFiles($Request){
 Write-Text $Request.TracePath ('Removal stage endpoint=21 session=99 stage=0 count=1 sequence=1 frequency=10000000 saturated=0' + "`n"+
  'Removal observation endpoint=21 session=99 stage=0 kind=0 sequence=1 qpc=101 has_status=0 status=STATUS_SUCCESS native_call=0'+"`n")
 Write-Text $Request.SummaryPath "Total Events Processed 24`nTotal Events Lost 0`nTotal Format Errors 0`nTotal Formats Unknown 0`n"
}
function New-ToolBoundary {
 @{
  Start={
   param($request)
   $script:fixture.Calls.Add($request)
   Check ($request.TimeoutSeconds-eq15-and$request.MaxLogBytes-eq1048576) 'Frozen finite policy changed.'
   $code='[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);[Console]::Write("harmless child");exit 0'
   if($request.Step-ceq'tracefmt'){
    $expected=@($script:fixture.Pins.Etl.Path,'-p',$script:fixture.Pins.Symbols.Directory,'-o',$request.TracePath)
    Check (($request.ArgumentList-join'|')-ceq($expected-join'|')) 'Wrong tracefmt arguments.'
    Write-DecodedFiles $request
    switch($script:fixture.Mode){
     'trace-fail'{$code='exit 3'}
     'mutate-etl'{[IO.File]::AppendAllText($script:fixture.Pins.Etl.Path,'mutation')}
     'bad-utf8'{[IO.File]::WriteAllBytes($request.TracePath,[byte[]](255,254,253))}
     'missing-summary'{[IO.File]::Delete($request.SummaryPath)}
     'reader-refusal'{Write-Text $request.SummaryPath 'malformed retained summary'}
    }
    $o=Start-FiniteTool -Executable $script:ps -ExpectedSha256 $script:psPin.Sha256 -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-Command',$code) -OutputDirectory $request.OutputDirectory -TimeoutSeconds 15 -MaxLogBytes 1048576
   } else {
    Check ($request.Step-ceq'associate'-and$request.ArgumentList[0]-ceq$script:fixture.Pins.Reader.Path) 'Wrong offline reader.'
    if($script:fixture.Mode-ceq'mutate-report'){[IO.File]::AppendAllText($script:fixture.Pins.ProbeReport.Path,' ')}
    if($script:fixture.Mode-ceq'mutate-trace'){[IO.File]::AppendAllText($request.TracePath,' ')}
    $o=Start-FiniteTool -Executable $request.Executable -ExpectedSha256 $request.ExpectedSha256 -ArgumentList $request.ArgumentList -OutputDirectory $request.OutputDirectory -TimeoutSeconds 15 -MaxLogBytes 1048576
   }
   $script:fixture.Owners.Add($o);$script:realOwners.Add($o)
   if($script:fixture.Mode-ceq'throw-after-launch'){$e=[InvalidOperationException]::new('mock dispatch observation failed');$e.Data['ToolOwner']=$o;throw $e}
   return $o
  }
  Observe={
   param($owner,$decode)
   Check ([Object]::ReferenceEquals($owner,$decode.ToolOwners[$decode.ToolOwners.Count-1])) 'Owner not retained before observation.'
   $r=Observe-FiniteTool $owner
   if($script:fixture.Mode-ceq'wrong-owner'){$r.Owner=[pscustomobject]@{}}
   if($script:fixture.Mode-ceq'null-exit'){$r.ExitCode=$null}
   if($script:fixture.Mode-ceq'timeout'){$owner.TimedOut=$true;$r=Get-FiniteToolReceipt $owner}
   if($script:fixture.Mode-ceq'log-cap'){$r.LogLimitExceeded=$true}
   return $r
  }
 }
}
function New-Decode($Fixture,[hashtable]$Boundary){New-RemovalTraceDecode -TraceSession $Fixture.Session -ManagedSession $Fixture.ManagedSession -Pins $Fixture.Pins -OutputDirectory (Join-Path $Fixture.Root 'decode') -ToolBoundary $Boundary}
function Set-ReleaseManifest($Fixture){
 # Shape copied from the actual managed-probe-release-pin.json. File content is
 # synthetic; the build-host path deliberately differs from the relocated probe.
 $build=@{SourceHead='26cd706f773f9bfb75e9572fc66594b5f932b918'
  BuildCommand='cargo build --locked --release -p moiren-windows-audio --example e2e_latency_probe'
  BuildExit=0;RustSourcesMatchSourceHead=$true;Files=@(
   @{Name='e2e_latency_probe.exe';Path='D:\build\target\release\examples\e2e_latency_probe.exe'
    Bytes=$Fixture.Pins.Probe.File.Bytes;Sha256=$Fixture.Pins.Probe.File.Sha256},
   @{Name='e2e_latency_probe.pdb';Path='D:\build\target\release\examples\e2e_latency_probe.pdb'
    Bytes=123;Sha256=('a'*64)})}
 $Fixture.Pins.Probe.SourceRevision=$build.SourceHead
 Write-Text $Fixture.Pins.Probe.BuildManifest.Path ($build|ConvertTo-Json -Depth 8)
 $Fixture.Pins.Probe.BuildManifest=Pin $Fixture.Pins.Probe.BuildManifest.Path
 return $build
}
