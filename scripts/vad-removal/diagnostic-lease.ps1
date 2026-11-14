#requires -Version 5.1
# One fixed writer handle excludes cooperating controllers across processes.
# The file is reused after close; its contents alone never establish ownership.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'task-boundary.ps1')

function New-RemovalDiagnosticLease {
    param([string]$Path='D:\MoirenTest\removal-613-controller.lock')
    Assert-RemovalTaskPath $Path
    $self=[Diagnostics.Process]::GetCurrentProcess()
    $config=ConvertTo-Json -InputObject ([ordered]@{Path=$Path;ControllerPid=$self.Id
        ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks}) -Compress
    [pscustomobject]@{Path=$Path;ControllerPid=$self.Id;ControllerCreationUtcTicks=$self.StartTime.ToUniversalTime().Ticks
        ConfigJson=$config;Attempted=$false;Owned=$false;Closed=$false;File=$null;RetainedFile=$null;RetainedHandle=$null
        Content=$null;Error=$null}
}

function Assert-RemovalLeaseController($Owner) {
    Assert-RemovalTaskHost
    $self=[Diagnostics.Process]::GetCurrentProcess()
    $config=ConvertTo-Json -InputObject ([ordered]@{Path=$Owner.Path;ControllerPid=$Owner.ControllerPid
        ControllerCreationUtcTicks=$Owner.ControllerCreationUtcTicks}) -Compress
    if($config -cne $Owner.ConfigJson -or $self.Id -ne $Owner.ControllerPid -or
        $self.StartTime.ToUniversalTime().Ticks -ne $Owner.ControllerCreationUtcTicks -or
        -not[Object]::ReferenceEquals($Owner.File,$Owner.RetainedFile)){throw 'Original lease controller/path/handle changed.'}
}

function Acquire-RemovalDiagnosticLease {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($Owner.Attempted -or $Owner.Closed){throw 'Lease acquisition already attempted; never steal/retry.'}
    $Owner.Attempted=$true
    try {
        Assert-RemovalLeaseController $Owner;Assert-RemovalTaskPath $Owner.Path
        if(-not(Test-Path -LiteralPath ([IO.Path]::GetDirectoryName($Owner.Path)) -PathType Container)){throw 'Existing fixed lease directory required.'}
        $Owner.File=[IO.File]::Open($Owner.Path,[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read)
        $Owner.RetainedFile=$Owner.File;$Owner.RetainedHandle=$Owner.File.SafeFileHandle;$Owner.Owned=$true
        # Retain the actual writer before any persistence can fail. No later
        # admission error closes it or permits another controller to enter.
        $Owner.Content=[Text.UTF8Encoding]::new($false).GetBytes($Owner.ConfigJson)
        $Owner.File.SetLength(0);$Owner.File.Write($Owner.Content,0,$Owner.Content.Length);$Owner.File.Flush($true)
        Assert-RemovalDiagnosticLease $Owner
    } catch {$Owner.Error=$_.Exception.Message;throw}
}

function Assert-RemovalDiagnosticLease {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    Assert-RemovalLeaseController $Owner;Assert-RemovalTaskPath $Owner.Path
    $f=$Owner.RetainedFile
    if(-not$Owner.Attempted -or -not$Owner.Owned -or $Owner.Closed -or $Owner.Error -or
        $null -eq $f -or $f -isnot [IO.FileStream] -or $null -eq $Owner.RetainedHandle -or
        $Owner.RetainedHandle.IsClosed -or $Owner.RetainedHandle.IsInvalid -or
        -not$f.CanRead -or -not$f.CanWrite -or $null -eq $Owner.Content -or
        $f.Length -ne $Owner.Content.Length -or $f.Length -gt 4096){throw 'Original live exclusive writer is missing or failed.'}
    $position=$f.Position
    try {
        $f.Position=0;$bytes=New-Object byte[] $Owner.Content.Length
        $read=$f.Read($bytes,0,$bytes.Length)
        if($read -ne $bytes.Length -or [Convert]::ToBase64String($bytes) -cne [Convert]::ToBase64String($Owner.Content)){
            throw 'Original exclusive lease evidence changed.'
        }
    } finally {$f.Position=$position}
}

function Release-RemovalDiagnosticLease {
    param([Parameter(Mandatory=$true,Position=0)]$Owner)
    $ErrorActionPreference='Stop'
    if($Owner.Closed){return}
    # Only controller/reference identity governs disposal of our own handle.
    # Expired baseline, failed collection or altered file bytes cannot require
    # reopening/stealing another owner to release this retained writer.
    Assert-RemovalLeaseController $Owner
    if($null -ne $Owner.RetainedFile){
        if($Owner.RetainedFile -isnot [IO.FileStream]){throw 'Original lease file type differs.'}
        $Owner.RetainedFile.Dispose()
    }
    $Owner.Owned=$false;$Owner.Closed=$true
}
