#requires -Version 5.1
Set-StrictMode -Version 2
$ErrorActionPreference='Stop'
$checks=New-Object Collections.Generic.List[object]
function Check([bool]$Value,[string]$Why){if(-not$Value){throw $Why}}
function Reject([scriptblock]$Action){$no=$false;try{&$Action|Out-Null}catch{$no=$true};Check $no 'Expected refusal.'}
function Case([string]$Name,[scriptblock]$Action){try{&$Action;$checks.Add(@{name=$Name;passed=$true})}catch{$checks.Add(@{name=$Name;passed=$false;error=$_.Exception.Message})}}
$library=Join-Path $PSScriptRoot 'diagnostic-lease.ps1'
Case 'exclusive-controller-lease-definitions-only' {
 Check (Test-Path -LiteralPath $library) 'Original writer-handle lease is missing.'
 . $library
 foreach($name in @('New-RemovalDiagnosticLease','Acquire-RemovalDiagnosticLease','Assert-RemovalDiagnosticLease','Release-RemovalDiagnosticLease')){
  Check ($null-ne(Get-Command $name -ErrorAction SilentlyContinue)) ('Missing '+$name)
 }
}
if(@($checks|Where-Object{-not$_.passed}).Count){$checks|ConvertTo-Json;Write-Output 'CHECKS=1 FAILED=1 NATIVE_AUDIO=false';exit 1}
. $library
if([Environment]::MachineName -ceq 'LAPTOP-NP49RG6H'){throw 'Lease tests run only on the development/CI host.'}
$root=Join-Path $PSScriptRoot ('logs\lease-'+[Guid]::NewGuid().ToString('N'))
$null=[IO.Directory]::CreateDirectory($root)
$leases=New-Object Collections.Generic.List[object]
function Lease([string]$Name) {
 $o=New-RemovalDiagnosticLease -Path (Join-Path $root $Name)
 $leases.Add($o);return $o
}
Case 'wrong-actual-host-refuses-before-opening-lease' {
 $o=Lease 'wrong-host.lock'
 Reject {Acquire-RemovalDiagnosticLease $o}
 Check (-not(Test-Path -LiteralPath $o.Path)-and$null-eq$o.File-and$o.Error) 'Wrong host touched a lease.'
}
function Assert-RemovalTaskHost {}
Case 'construction-does-not-create-or-acquire-a-file' {
 $o=Lease 'construction.lock'
 Check (-not$o.Attempted-and-not$o.Owned-and-not$o.Closed-and$null-eq$o.File-and-not(Test-Path $o.Path)) 'Constructor acquired resources.'
}
Case 'original-open-writer-handle-excludes-a-second-controller' {
 $o=Lease 'exclusive.lock';Acquire-RemovalDiagnosticLease $o;Assert-RemovalDiagnosticLease $o
 $other=Lease 'exclusive.lock';Reject {Acquire-RemovalDiagnosticLease $other}
 Check ($o.Owned-and-not$o.Closed-and$null-eq$other.File-and$other.Error) 'Contender stole/discarded original lease.'
 Assert-RemovalDiagnosticLease $o
 Reject {Acquire-RemovalDiagnosticLease $other}
}
Case 'closed-lease-file-is-reused-without-deleting-or-rotating-paths' {
 $o=Lease 'reused.lock';Acquire-RemovalDiagnosticLease $o;$path=$o.Path
 Release-RemovalDiagnosticLease $o
 Check ($o.Closed-and-not$o.Owned-and(Test-Path $path)) 'Release removed the fixed lease path or retained writer.'
 $next=Lease 'reused.lock';Acquire-RemovalDiagnosticLease $next;Assert-RemovalDiagnosticLease $next
 Reject {Acquire-RemovalDiagnosticLease $o}
 Release-RemovalDiagnosticLease $next
}
Case 'exact-controller-path-handle-and-original-content-are-revalidated' {
 foreach($mode in @('controller','creation','path','handle','content')){
  $o=Lease ($mode+'.lock');Acquire-RemovalDiagnosticLease $o;$original=$o.RetainedFile
  switch($mode){
   'controller'{$o.ControllerPid++};'creation'{$o.ControllerCreationUtcTicks++}
   'path'{$o.Path=Join-Path $root 'foreign.lock'}
   'handle'{$o.File=[pscustomobject]@{fixture='foreign'}}
   'content'{$original.Position=0;$original.WriteByte(32);$original.Flush($true)}
  }
  Reject {Assert-RemovalDiagnosticLease $o}
  Check (-not$original.SafeFileHandle.IsClosed) 'Assertion closed an unresolved original writer.'
  # Test teardown owns this exact real fixture handle; it never opens a foreign file.
  $original.Dispose()
 }
}
Case 'retained-cleanup-does-not-depend-on-fresh-readiness-or-unmodified-content' {
 $o=Lease 'cleanup.lock';Acquire-RemovalDiagnosticLease $o
 $o.File.Position=0;$o.File.WriteByte(32);$o.File.Flush($true)
 Reject {Assert-RemovalDiagnosticLease $o}
 Release-RemovalDiagnosticLease $o
 Check ($o.Closed-and-not$o.Owned-and$o.RetainedHandle.IsClosed) 'Failed admission prevented release of our own writer.'
}
Case 'release-never-closes-a-replaced-public-handle' {
 $o=Lease 'replace.lock';Acquire-RemovalDiagnosticLease $o;$original=$o.RetainedFile
 $foreign=[IO.MemoryStream]::new();$o.File=$foreign
 Reject {Release-RemovalDiagnosticLease $o}
 Check ($foreign.CanWrite-and-not$original.SafeFileHandle.IsClosed) 'Replacement caused foreign/original premature cleanup.'
 $foreign.Dispose();$original.Dispose()
}
foreach($o in $leases){
 if($null-ne$o.RetainedFile){$o.RetainedFile.Dispose()}
}
$checks|ConvertTo-Json -Depth 6
$failed=@($checks|Where-Object{-not$_.passed}).Count
Write-Output ('CHECKS='+$checks.Count+' FAILED='+$failed+' FILE_WRITER_EXCLUSION=actual NATIVE_AUDIO=false')
if($failed){exit 1}
