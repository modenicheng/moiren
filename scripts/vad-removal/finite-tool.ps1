#requires -Version 5.1
# Evidence-only functions. Dot-sourcing launches nothing; callers pin exact tools/argv.
Set-StrictMode -Version 2

function Quote-FiniteToolArgument([AllowEmptyString()][string]$Value) {
    $text=[Text.StringBuilder]::new();$null=$text.Append('"');$slashes=0
    foreach($character in $Value.ToCharArray()){
        if($character -eq '\'){$slashes++;continue}
        if($character -eq '"'){$null=$text.Append(('\'*($slashes*2+1)))}
        else {$null=$text.Append(('\'*$slashes))}
        $null=$text.Append($character);$slashes=0
    }
    $null=$text.Append(('\'*($slashes*2))).Append('"');$text.ToString()
}

function Assert-FiniteToolPath([string]$Path,[bool]$File) {
    if($Path -notmatch '^[A-Za-z]:\\' -or $Path -match '[\x00\r\n"*?]' -or
        $Path.Substring(2).Contains(':') -or [IO.Path]::GetFullPath($Path) -cne $Path){throw 'Canonical local absolute path required.'}
    $cursor=$Path
    while($cursor){
        if(Test-Path -LiteralPath $cursor){
            $item=Get-Item -LiteralPath $cursor -Force -ErrorAction Stop
            if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Reparse path refused.'}
            if($cursor -cne $Path -and -not$item.PSIsContainer){throw 'Non-directory ancestor.'}
        }
        $cursor=[IO.Path]::GetDirectoryName($cursor)
    }
    if($File -and (-not(Test-Path -LiteralPath $Path -PathType Leaf) -or [IO.Path]::GetExtension($Path) -ine '.exe')){
        throw 'Existing exact Windows executable required.'
    }
}

function Initialize-FiniteToolSink {
    if('Moiren.RemovalFiniteToolV1.Budget' -as [type]){return}
    # Adapted from scripts/vad-collection-run.ps1 Budget/Sink. One shared
    # reservation cap bounds both files; after overflow discard while draining
    # pipes instead of blocking a still-owned child or killing it automatically.
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
namespace Moiren.RemovalFiniteToolV1 {
    public sealed class Budget {
        private readonly object gate = new object();
        private readonly long limit;
        private long reserved;
        private volatile bool exceeded;
        public Budget(long limit) { this.limit = limit; }
        public bool Exceeded { get { return exceeded; } }
        public long Reserved { get { lock (gate) { return reserved; } } }
        internal int Reserve(int count) {
            lock (gate) {
                int allowed = (int)Math.Min((long)count, limit - reserved);
                reserved += allowed;
                if (allowed != count) exceeded = true;
                return allowed;
            }
        }
    }
    public sealed class Sink : Stream {
        private readonly Stream destination;
        private readonly Budget budget;
        private volatile bool failed;
        public Sink(Stream destination, Budget budget) { this.destination=destination; this.budget=budget; }
        public bool Failed { get { return failed; } }
        public override bool CanRead { get { return false; } }
        public override bool CanSeek { get { return false; } }
        public override bool CanWrite { get { return true; } }
        public override long Length { get { throw new NotSupportedException(); } }
        public override long Position { get { throw new NotSupportedException(); } set { throw new NotSupportedException(); } }
        public override void Flush() { destination.Flush(); }
        public override int Read(byte[] b,int o,int c) { throw new NotSupportedException(); }
        public override long Seek(long o,SeekOrigin w) { throw new NotSupportedException(); }
        public override void SetLength(long v) { throw new NotSupportedException(); }
        public override void Write(byte[] b,int o,int c) {
            int allowed=budget.Reserve(c);
            if (allowed == 0 || failed) return;
            try { destination.Write(b,o,allowed); } catch { failed=true; }
        }
        public override async Task WriteAsync(byte[] b,int o,int c,CancellationToken token) {
            int allowed=budget.Reserve(c);
            if (allowed == 0 || failed) return;
            try { await destination.WriteAsync(b,o,allowed,token).ConfigureAwait(false); } catch { failed=true; }
        }
    }
}
'@
}

function Add-FiniteToolError($Owner,[string]$Message) {
    if($Owner.Errors.Count -lt 16 -and -not$Owner.Errors.Contains($Message)){$Owner.Errors.Add($Message)}
}

function Start-FiniteTool {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory=$true)][string]$Executable,
        [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExpectedSha256,
        [Parameter(Mandatory=$true)][string]$OutputDirectory,
        [AllowEmptyCollection()][AllowEmptyString()][string[]]$ArgumentList=@(),
        [ValidateRange(0.01,15)][double]$TimeoutSeconds=15,
        [ValidateRange(1,16777216)][long]$MaxLogBytes=1048576,
        [scriptblock]$ProcessFactory
    )
    Assert-FiniteToolPath $Executable $true;Assert-FiniteToolPath $OutputDirectory $false
    if(Test-Path -LiteralPath $OutputDirectory){throw 'Create-new output directory required.'}
    if((Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash -ine $ExpectedSha256){throw 'Executable hash mismatch.'}
    $quoted=foreach($arg in $ArgumentList){
        if($null -eq $arg -or $arg -match '[\x00\r\n]'){throw 'Unsafe null/control argument.'}
        Quote-FiniteToolArgument $arg
    }
    $line=$quoted -join ' '
    if($Executable.Length+$line.Length+3 -gt 32766){throw 'Windows command line too long.'}
    Initialize-FiniteToolSink
    $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=$Executable;$start.Arguments=$line
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.WorkingDirectory=[IO.Path]::GetDirectoryName($Executable)
    $start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    $owner=[pscustomobject]@{
        Process=$null;Handle=[IntPtr]::Zero;ProcessId=$null;CreationUtcTicks=$null;CreationUtc=$null
        LaunchAttempted=$false;LaunchUnknown=$false;Started=$false;IdentityConfirmed=$false
        ExitedConfirmed=$false;ExitCode=$null;TimedOut=$false;Closed=$false;OutputStable=$false
        CompletionObservedMilliseconds=$null
        DeadlineMilliseconds=[long][Math]::Ceiling($TimeoutSeconds*1000);Clock=$null
        Executable=$Executable;ExecutableSha256=$ExpectedSha256;Arguments=@($ArgumentList)
        StdoutPath=(Join-Path $OutputDirectory 'stdout.log');StderrPath=(Join-Path $OutputDirectory 'stderr.log')
        Files=@();Copies=@();Flushes=@();Sinks=@();Budget=[Moiren.RemovalFiniteToolV1.Budget]::new($MaxLogBytes)
        Errors=New-Object Collections.Generic.List[string]
    }
    try {
        $null=[IO.Directory]::CreateDirectory($OutputDirectory)
        foreach($path in @($owner.StdoutPath,$owner.StderrPath)){
            $file=[IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
            $owner.Files+=,$file
        }
        $owner.Process=if($null -ne $ProcessFactory){&$ProcessFactory}else{[Diagnostics.Process]::new()}
        $owner.Process.StartInfo=$start
        $owner.Clock=[Diagnostics.Stopwatch]::StartNew()
        $owner.LaunchAttempted=$true;$owner.LaunchUnknown=$true
        if(-not$owner.Process.Start()){throw 'Process.Start did not confirm dispatch.'}
        $owner.Started=$true;$owner.LaunchUnknown=$false
        # Retain the original Process/CreateProcess handle; never reacquire by PID.
        $owner.Handle=$owner.Process.Handle
        $owner.ProcessId=$owner.Process.Id
        $created=$owner.Process.StartTime.ToUniversalTime()
        $owner.CreationUtcTicks=$created.Ticks;$owner.CreationUtc=$created.ToString('o')
        if($owner.Handle -isnot [IntPtr] -or $owner.Handle -eq [IntPtr]::Zero -or
            $owner.ProcessId -isnot [int] -or $owner.ProcessId -le 0 -or
            $owner.CreationUtcTicks -isnot [long] -or $owner.CreationUtcTicks -le 0){throw 'Process identity unavailable.'}
        $owner.IdentityConfirmed=$true
        $streams=@($owner.Process.StandardOutput.BaseStream,$owner.Process.StandardError.BaseStream)
        for($i=0;$i -lt 2;$i++){
            $sink=[Moiren.RemovalFiniteToolV1.Sink]::new($owner.Files[$i],$owner.Budget)
            $owner.Sinks+=,$sink
            $owner.Copies+=,$streams[$i].CopyToAsync($sink,4096)
        }
    } catch {Add-FiniteToolError $owner 'Launch/identity/output setup failed; retain exact owner, never relaunch.'}
    return $owner
}

function Get-FiniteToolReceipt($Owner) {
    $limit=$Owner.Budget.Exceeded
    $success=($Owner.ExitedConfirmed -and $Owner.ExitCode -eq 0 -and $Owner.OutputStable -and
        -not$Owner.TimedOut -and -not$limit -and $Owner.Errors.Count -eq 0)
    [pscustomobject]@{Owner=$Owner;ProcessId=$Owner.ProcessId;CreationUtcTicks=$Owner.CreationUtcTicks
        ExitedConfirmed=$Owner.ExitedConfirmed;ExitCode=$Owner.ExitCode;TimedOut=$Owner.TimedOut
        CompletionObservedMilliseconds=$Owner.CompletionObservedMilliseconds
        NeedsObservation=(-not$Owner.ExitedConfirmed -or -not$Owner.OutputStable)
        OutputStable=$Owner.OutputStable;LogLimitExceeded=$limit;Success=$success
        StdoutPath=$Owner.StdoutPath;StderrPath=$Owner.StderrPath;Errors=@($Owner.Errors.ToArray())}
}

function Observe-FiniteTool {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner,[ValidateRange(0,15000)][int]$ObservationMilliseconds=0)
    if($Owner.Closed){throw 'Exact process owner already closed.'}
    if(-not$Owner.IdentityConfirmed){return Get-FiniteToolReceipt $Owner}
    # Deadline acceptance uses the first fully observed exit/drain/flush, not
    # wall time on a later receipt read. First observation at deadline is late.
    if($null -ne $Owner.CompletionObservedMilliseconds){return Get-FiniteToolReceipt $Owner}
    # A later explicit observation can prove natural exit, never erase timeout.
    $end=if($ObservationMilliseconds -gt 0){$Owner.Clock.ElapsedMilliseconds+$ObservationMilliseconds}else{$Owner.DeadlineMilliseconds}
    try {
        while(-not$Owner.ExitedConfirmed){
            $elapsed=$Owner.Clock.ElapsedMilliseconds
            if($elapsed -ge $Owner.DeadlineMilliseconds){$Owner.TimedOut=$true}
            if($Owner.Process.WaitForExit(0)){
                $Owner.ExitedConfirmed=$true;$Owner.ExitCode=$Owner.Process.ExitCode;break
            }
            $left=$end-$elapsed
            if($left -le 0){break}
            $null=$Owner.Process.WaitForExit([int][Math]::Min(25,$left))
        }
        if($Owner.ExitedConfirmed){
            # No unbounded WaitForExit(): inherited descendant pipe handles may
            # outlive the exact child, so each drain shares this observation end.
            $drained=$Owner.Copies.Count -eq 2
            foreach($copy in $Owner.Copies){
                $left=[int][Math]::Max(0,[Math]::Min(15000,$end-$Owner.Clock.ElapsedMilliseconds))
                if(-not$copy.IsCompleted -and -not$copy.Wait($left)){$drained=$false;continue}
                $null=$copy.GetAwaiter().GetResult()
            }
            if($drained){
                # Retain each pending flush exactly once. Stream flush completion
                # is not a disk-durability or descendant-process attestation.
                while($Owner.Flushes.Count -lt $Owner.Files.Count){
                    $Owner.Flushes+=,$Owner.Files[$Owner.Flushes.Count].FlushAsync()
                }
                $flushed=$Owner.Flushes.Count -eq 2
                foreach($flush in $Owner.Flushes){
                    $left=[int][Math]::Max(0,[Math]::Min(15000,$end-$Owner.Clock.ElapsedMilliseconds))
                    if(-not$flush.IsCompleted -and -not$flush.Wait($left)){$flushed=$false;continue}
                    $null=$flush.GetAwaiter().GetResult()
                }
                if($flushed){
                    $sinkFailed=@($Owner.Sinks|Where-Object{$_.Failed}).Count -gt 0
                    if($sinkFailed){Add-FiniteToolError $Owner 'Output persistence failed.'}
                    $Owner.OutputStable=(-not$Owner.Budget.Exceeded -and -not$sinkFailed)
                    $Owner.CompletionObservedMilliseconds=$Owner.Clock.ElapsedMilliseconds
                }
            }
        }
    } catch {Add-FiniteToolError $Owner 'Exact process observation/output drain failed.'}
    $observed=if($null -ne $Owner.CompletionObservedMilliseconds){$Owner.CompletionObservedMilliseconds}else{$Owner.Clock.ElapsedMilliseconds}
    if($observed -ge $Owner.DeadlineMilliseconds){$Owner.TimedOut=$true}
    return Get-FiniteToolReceipt $Owner
}

function Close-FiniteToolOwner {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    if($Owner.Closed){return}
    # A throwing Start or missing identity cannot prove absence/exit. Keep its
    # exact object and files; this helper has no unsafe force-close/kill escape.
    if($Owner.LaunchAttempted -and (-not$Owner.ExitedConfirmed -or $Owner.Copies.Count -ne 2 -or
        @($Owner.Copies|Where-Object{-not$_.IsCompleted}).Count -or
        @($Owner.Flushes|Where-Object{-not$_.IsCompleted}).Count)){
        throw 'Unresolved exact owner/pipes/flush retained; no kill/dispose/relaunch.'
    }
    foreach($file in $Owner.Files){$file.Dispose()}
    if($null -ne $Owner.Process){$Owner.Process.Dispose()}
    $Owner.Closed=$true
}
