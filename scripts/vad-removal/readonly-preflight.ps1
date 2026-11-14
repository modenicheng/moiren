#requires -Version 5.1
# Definitions only. Native execution remains user-reboot gated in the caller.
Set-StrictMode -Version 2

function Read-RemovalUptime {
    if(-not('Moiren.RemovalBootV1' -as [type])){
        Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; namespace Moiren { public static class RemovalBootV1 { [DllImport("kernel32.dll")] public static extern UInt64 GetTickCount64(); } }'
    }
    return [Moiren.RemovalBootV1]::GetTickCount64()
}

function New-RemovalBootObservation([DateTimeOffset]$Boot,[DateTimeOffset]$Observed,[uint64]$Uptime) {
    $delta=($Observed-$Boot).TotalMilliseconds
    @{BootUtc=$Boot.ToUniversalTime().ToString('o');ObservedUtc=$Observed.ToUniversalTime().ToString('o')
      UptimeMilliseconds=$Uptime;BootAfterDeployment=($Boot -gt [DateTimeOffset]'2026-11-07T16:10:12.358563Z')
      ClockConsistent=($delta -ge 0 -and [Math]::Abs($delta-[double]$Uptime) -le 5000)}
}

function New-RemovalIdentity([string]$HostName,[string]$Sid,[int]$Session) {
    if($HostName -cne 'LAPTOP-NP49RG6H' -or $Sid -cne 'S-1-5-21-3080296863-1242338503-1346362208-1001' -or
        $Session -lt 0 -or $Session -gt 65535){throw 'Wrong target identity or unknown session.'}
    @{host=$HostName;sid=$Sid;session=$Session}
}

function Read-RemovalTargetIdentity {
    New-RemovalIdentity $env:COMPUTERNAME ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value) ([Diagnostics.Process]::GetCurrentProcess().SessionId)
}

function Convert-RemovalProcesses([AllowEmptyCollection()][object[]]$Rows,[int]$CollectorPid) {
    $ErrorActionPreference='Stop'
    $external=New-Object Collections.Generic.List[object]
    $native=New-Object Collections.Generic.List[object]
    $seen=@{}
    foreach($p in $Rows){
        if($p.ProcessId -eq $CollectorPid){continue}
        $protected=$p.Name -match '^(obs64|QQMusic|powershell|pwsh)\.exe$'
        $busy=$p.Name -match '^(cargo|rustc|rust-lld|lld-link|link|cl|MSBuild|ninja|cmake|moiren-app.*|e2e_latency_probe.*|vad_native.*|vad_integration.*|w00_probe.*|ApplicationLoopback.*|DesktopSupervisor|python|pythonw)\.exe$'
        if(-not $protected -and -not $busy){continue}
        if($null -eq $p.CreationDate -or $p.ExecutablePath -isnot [string] -or -not $p.ExecutablePath -or
            $seen.ContainsKey([string]$p.ProcessId)){throw 'Unknown/duplicate protected or native process identity.'}
        $seen[[string]$p.ProcessId]=$true
        $row=[pscustomobject][ordered]@{Pid=$p.ProcessId;Name=$p.Name;Session=$p.SessionId
            CreationUnixMs=([DateTimeOffset]$p.CreationDate).ToUnixTimeMilliseconds();ExecutablePath=$p.ExecutablePath}
        $fixedObs=$p.Name -ieq 'obs64.exe' -and $p.ExecutablePath -ieq 'D:\MoirenTest\20261101-clean-comb-probe-1320\obs-portable\bin\64bit\obs64.exe'
        if($busy -or $fixedObs){$native.Add($row)}else{$external.Add($row)}
    }
    @{ExternalOwners=@($external.ToArray()|Sort-Object Pid);NativeOwners=@($native.ToArray()|Sort-Object Pid)}
}

function Convert-RemovalTasks([AllowEmptyCollection()][object[]]$Rows) {
    $pending=@(foreach($task in $Rows){
        if($task.TaskName -notlike 'Moiren*'){continue}
        $state=[string]$task.State
        if($state -cnotin @('Ready','Disabled')){
            [pscustomobject][ordered]@{Path=$task.TaskPath;Name=$task.TaskName;State=$state}
        }
    })
    return $pending|Sort-Object Path,Name
}

function Assert-RemovalCatalogErrors($Object) {
    $e=$Object.errors
    if($e -isnot [Array] -or $e.Count -ne 0){throw 'Missing/malformed/nonempty catalog errors.'}
}

function Convert-RemovalCatalog($Catalog) {
    if($Catalog.schema_version -ne 1){throw 'Unsupported catalog schema.'}
    Assert-RemovalCatalogErrors $Catalog;Assert-RemovalCatalogErrors $Catalog.before
    if($Catalog.before.defaults -isnot [Array] -or $Catalog.before.defaults.Count -ne 6){throw 'Incomplete default roles.'}
    $defaults=New-Object Collections.Generic.List[object]
    foreach($flow in @('render','capture')){
        foreach($role in @('console','multimedia','communications')){
            $rows=@($Catalog.before.defaults|Where-Object{$_.flow -ceq $flow -and $_.role -ceq $role})
            if($rows.Count -ne 1 -or $rows[0].id -isnot [string] -or -not $rows[0].id){throw 'Unknown/duplicate default role.'}
            $defaults.Add([pscustomobject][ordered]@{flow=$flow;role=$role;id=$rows[0].id})
        }
    }
    $render=New-Object Collections.Generic.List[object];$seen=@{}
    foreach($e in $Catalog.before.endpoints){
        Assert-RemovalCatalogErrors $e
        if($e.flow -cne 'render'){continue}
        $numeric=$e.volume_scalar -is [double] -or $e.volume_scalar -is [single] -or $e.volume_scalar -is [decimal] -or $e.volume_scalar -is [int] -or $e.volume_scalar -is [long]
        if($e.id -isnot [string] -or -not $e.id -or $seen.ContainsKey($e.id) -or -not $numeric -or
            [double]::IsNaN([double]$e.volume_scalar) -or $e.volume_scalar -lt 0 -or $e.volume_scalar -gt 1 -or $e.muted -isnot [bool]){throw 'Unknown render identity/control.'}
        $seen[$e.id]=$true
        $render.Add([pscustomobject][ordered]@{id=$e.id;volume_scalar=$e.volume_scalar;muted=$e.muted})
    }
    if($render.Count -eq 0){throw 'No render controls observed.'}
    @{Defaults=@($defaults.ToArray()|Sort-Object flow,role);Render=@($render.ToArray()|Sort-Object id)}
}

function Invoke-RemovalReadTool([string]$Name,[string]$Executable,[string]$Sha256,[string[]]$Argv,
    [string]$EvidenceDirectory,[Collections.Generic.List[object]]$Owners) {
    $owner=Start-FiniteTool -Executable $Executable -ExpectedSha256 $Sha256 -ArgumentList $Argv `
        -OutputDirectory (Join-Path $EvidenceDirectory $Name) -TimeoutSeconds 15 -MaxLogBytes 4194304
    $Owners.Add($owner)
    $result=Observe-FiniteTool $owner
    if(-not $result.Success){throw ('Read tool failed/unknown; retained exact owner: '+$Name)}
    Close-FiniteToolOwner $owner
    return $result
}

function Get-RemovalReadTools {
    @{
        SignTool=@{Path='D:\MoirenTest\20261029-0118a54\signtool.exe'
            Sha256='C8AC2561B1E75031EE864CCA3FB2B4CD06630E3EEA42D86AB80A573BCF8B9472'}
        Catalog=@{Path='D:\MoirenTest\20261029-0118a54\bin\w00_probe.exe'
            Sha256='58552AEC72650250FA0C01A309B6B885561A38F925842175C1C5B0EEEB0F081A'}
        BcdEdit=@{Path=(Join-Path $env:SystemRoot 'System32\bcdedit.exe');Sha256=$null}
    }
}

function Resolve-RemovalReadTools {
    [CmdletBinding()]
    param($ReadToolPins=$null)
    $ErrorActionPreference='Stop'
    $defaults=Get-RemovalReadTools;$resolved=@{}
    foreach($key in @('SignTool','Catalog','BcdEdit')){
        $expected=$defaults[$key];$path=$expected.Path
        Assert-FiniteToolPath $path $true
        $item=Get-Item -LiteralPath $path -Force -ErrorAction Stop
        if($item.PSIsContainer -or $item.Length -lt 1 -or $item.Length -gt 268435456){
            throw ('Read tool identity has invalid file type/length: '+$key)
        }
        $pin=$null
        if($null -ne $ReadToolPins){
            # A full input Tools object may include decoder tools. Only these
            # three exact read-tool identities have authority in this collector.
            $pin=if($ReadToolPins -is [Collections.IDictionary]){$ReadToolPins[$key]}else{$ReadToolPins.$key}
            if($null -eq $pin -or $pin.Path -isnot [string] -or $pin.Path -cne $path -or
                ($pin.Bytes -isnot [int] -and $pin.Bytes -isnot [long]) -or $pin.Bytes -ne $item.Length -or
                $pin.Sha256 -isnot [string] -or $pin.Sha256 -cnotmatch '\A[A-Fa-f0-9]{64}\z'){
                throw ('Read tool pin path/length/hash is missing or malformed: '+$key)
            }
        }
        $hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256 -ErrorAction Stop).Hash
        if(($null -ne $expected.Sha256 -and $hash -ine $expected.Sha256) -or
            ($null -ne $pin -and $hash -ine $pin.Sha256)){
            throw ('Read tool identity differs from original pin: '+$key)
        }
        $resolved[$key]=[pscustomobject]@{Path=$path;Bytes=[long]$item.Length;Sha256=$hash.ToUpperInvariant()}
    }
    return $resolved
}

function Read-RemovalPreflight {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$EvidenceDirectory,
        [Parameter(Mandatory=$true)][AllowEmptyCollection()][Collections.Generic.List[object]]$ToolOwners,
        $ReadToolPins=$null)
    $ErrorActionPreference='Stop'
    $started=[DateTimeOffset]::UtcNow
    $identity=Read-RemovalTargetIdentity
    Assert-FiniteToolPath $EvidenceDirectory $false
    if(Test-Path -LiteralPath $EvidenceDirectory){throw 'Fresh evidence directory required.'}
    $null=[IO.Directory]::CreateDirectory($EvidenceDirectory)
    $os=Get-CimInstance Win32_OperatingSystem -OperationTimeoutSec 5 -ErrorAction Stop
    $boot=[DateTimeOffset]$os.LastBootUpTime
    $initial=New-RemovalBootObservation $boot ([DateTimeOffset]::UtcNow) (Read-RemovalUptime)
    if(-not $initial.BootAfterDeployment -or -not $initial.ClockConsistent){throw 'User reboot not established; no subsequent tool/device collection.'}
    # Freeze/validate all three before the first signature tool can launch. The
    # finite launcher checks the frozen expected hash again at each dispatch.
    $readTools=Resolve-RemovalReadTools -ReadToolPins $ReadToolPins
    $base='D:\MoirenTest\20261107-233656-removal-diagnostic'
    $signtool=$readTools.SignTool.Path;$sigHash=$readTools.SignTool.Sha256
    $catalog=$readTools.Catalog.Path;$catalogHash=$readTools.Catalog.Sha256
    $signer='7F17CCDCA257D6ABC84255DD0D1854E93B9B5C1D'
    foreach($pin in @(
        @((Join-Path $base 'package-manifest.json'),'DEF621D149EF43753D27B8CE2A8DFA1237E6D8B106D369931A50F07E24AC1317'),
        @('D:\MoirenTest\20261107-direct-remote-driver-1755\package-manifest.json','94883DAC472B67A3344D22D7DA10B355ECD6565C87BCD022DFC10D56D1C44EAF')
    )){
        Assert-FiniteToolPath $pin[0] $false
        if((Get-FileHash -LiteralPath $pin[0]).Hash -cne $pin[1]){throw 'Candidate or rollback manifest changed.'}
    }
    $expected=@{
        'package-613'=@{
            'moirenvad.cat'=@(2514,'A6171C7C474667BF4A17B999D3187A325D509BA742EF89BA7322C8384E181E20')
            'MoirenVad.inf'=@(1799,'24574ED3BAE942EF7ED91601513F8F0B181272BF1F15D2067F0F6EE2014F827C')
            'MoirenVad.sys'=@(152472,'0A1DFF698923B2DF39D0625979D3CD1AEE486228396C1BF601EADEA565EB3B76')}
        'rollback-612'=@{
            'moirenvad.cat'=@(2514,'AD5090790C6B359A6D169A7B3206FF3EA6F0CE026507087E80F29A1BEF3A83BA')
            'MoirenVad.inf'=@(1799,'BB526217B87462C8A986A62FC3348959C83E28CE97862C47D496D2F4CF84624D')
            'MoirenVad.sys'=@(147352,'B25CA2E8DB8DD3E98D9B90AA6CF19822C879E6CA13D52B35D160CEF2056D1737')}
    }
    foreach($package in @('package-613','rollback-612')){
        $directory=Join-Path $base $package
        foreach($name in @('moirenvad.cat','MoirenVad.inf','MoirenVad.sys')){
            $path=Join-Path $directory $name;Assert-FiniteToolPath $path $false
            $item=Get-Item -LiteralPath $path -Force -ErrorAction Stop;$pin=$expected[$package][$name]
            if($item.PSIsContainer -or $item.Length -ne $pin[0] -or (Get-FileHash -LiteralPath $path).Hash -cne $pin[1]){throw 'Package/rollback pin differs.'}
        }
        foreach($name in @('MoirenVad.sys','moirenvad.cat')){
            $path=Join-Path $directory $name;$signature=Get-AuthenticodeSignature -LiteralPath $path
            if($signature.Status -ne 'Valid' -or $null -eq $signature.SignerCertificate -or $signature.SignerCertificate.Thumbprint -cne $signer){throw 'Signature/signer differs.'}
            $null=Invoke-RemovalReadTool ($package+'-signature-'+$name) $signtool $sigHash @('verify','/pa','/v',$path) $EvidenceDirectory $ToolOwners
        }
        foreach($name in @('MoirenVad.sys','MoirenVad.inf')){
            $null=Invoke-RemovalReadTool ($package+'-member-'+$name) $signtool $sigHash @('verify','/pa','/v','/c',(Join-Path $directory 'moirenvad.cat'),(Join-Path $directory $name)) $EvidenceDirectory $ToolOwners
        }
    }
    $instance='ROOT\MEDIA\0001'
    $services=@(Get-CimInstance Win32_SystemDriver -Filter "Name='MoirenVad'" -OperationTimeoutSec 5 -ErrorAction Stop)
    if($services.Count -ne 1){throw 'Expected one driver service.'}
    $device=Get-PnpDevice -InstanceId $instance -ErrorAction Stop
    $properties=@{}
    foreach($key in @('HardwareIds','ProblemCode','DriverInfPath','DriverVersion')){
        $properties[$key]=(Get-PnpDeviceProperty -InstanceId $instance -KeyName ('DEVPKEY_Device_'+$key) -ErrorAction Stop).Data
    }
    if(@($properties.HardwareIds) -notcontains 'ROOT\MoirenVad'){throw 'Wrong root hardware identity.'}
    $sys=[Environment]::ExpandEnvironmentVariables(([string]$services[0].PathName).Trim('"'))
    if($sys.StartsWith('\??\')){$sys=$sys.Substring(4)}
    if($sys.StartsWith('\SystemRoot\',[StringComparison]::OrdinalIgnoreCase)){$sys=Join-Path $env:SystemRoot $sys.Substring(12)}
    Assert-FiniteToolPath $sys $false
    if($properties.DriverInfPath -cne 'oem207.inf'){throw 'Unexpected installed INF binding.'}
    $driver=[ordered]@{Version=$properties.DriverVersion;Inf=$properties.DriverInfPath;State=$services[0].State
        InstanceId=$device.InstanceId;PnpStatus=$device.Status;ProblemCode=$properties.ProblemCode;SysPath=$sys
        SysSha256=(Get-FileHash -LiteralPath $sys).Hash
        InfSha256=(Get-FileHash -LiteralPath (Join-Path $env:SystemRoot 'INF\oem207.inf')).Hash}
    if($driver.Version -cne '3.1.1.613' -or $driver.State -cne 'Running' -or $driver.PnpStatus -cne 'OK' -or
        $driver.InstanceId -ine $instance -or $null -eq $driver.ProblemCode -or $driver.ProblemCode -ne 0 -or
        $driver.SysSha256 -cne $expected['package-613']['MoirenVad.sys'][1] -or
        $driver.InfSha256 -cne $expected['package-613']['MoirenVad.inf'][1]){throw 'Unhealthy or wrong installed driver binding.'}
    $r=Invoke-RemovalReadTool 'catalog' $catalog $catalogHash @('--list') $EvidenceDirectory $ToolOwners
    $controls=Convert-RemovalCatalog ([IO.File]::ReadAllText($r.StdoutPath)|ConvertFrom-Json)
    $bcd=$readTools.BcdEdit.Path;$bcdHash=$readTools.BcdEdit.Sha256
    $r=Invoke-RemovalReadTool 'bcd' $bcd $bcdHash @('/enum','{current}') $EvidenceDirectory $ToolOwners
    $guard=@(Get-CimInstance -Namespace root\Microsoft\Windows\DeviceGuard -ClassName Win32_DeviceGuard -OperationTimeoutSec 5 -ErrorAction Stop)
    if($guard.Count -ne 1){throw 'Unknown DeviceGuard state.'}
    $security=[ordered]@{SecureBoot=(Confirm-SecureBootUEFI);Vbs=$guard[0].VirtualizationBasedSecurityStatus
        CodeIntegrity=$guard[0].CodeIntegrityPolicyEnforcementStatus;UserCodeIntegrity=$guard[0].UsermodeCodeIntegrityPolicyEnforcementStatus
        Configured=@($guard[0].SecurityServicesConfigured|Sort-Object);Running=@($guard[0].SecurityServicesRunning|Sort-Object)
        Bcd=(Get-FileHash -LiteralPath $r.StdoutPath).Hash}
    if($security.SecureBoot -isnot [bool] -or $null -eq $security.Vbs -or $null -eq $security.CodeIntegrity -or
        $null -eq $security.UserCodeIntegrity -or $security.Configured.Count -eq 0 -or $security.Running.Count -eq 0){throw 'Incomplete security observation.'}
    $tasks=@(Convert-RemovalTasks @(Get-ScheduledTask -ErrorAction Stop))
    $children=@(Get-PnpDevice -PresentOnly -ErrorAction Stop|Where-Object{$_.InstanceId -like 'MOIREN\VAD_RAW\*'}|ForEach-Object{
        [pscustomobject][ordered]@{InstanceId=$_.InstanceId;Status=$_.Status}
    }|Sort-Object InstanceId)
    $endBoot=[DateTimeOffset](Get-CimInstance Win32_OperatingSystem -OperationTimeoutSec 5 -ErrorAction Stop).LastBootUpTime
    # Capture process identities last; preserve the real enumeration interval
    # instead of stamping an older result with a later safety-collection time.
    $processStarted=[DateTimeOffset]::UtcNow
    $processRows=@(Get-CimInstance Win32_Process -OperationTimeoutSec 5 -ErrorAction Stop)
    $processEnded=[DateTimeOffset]::UtcNow
    $processes=Convert-RemovalProcesses $processRows $PID
    $observed=$processEnded;$bootRecord=New-RemovalBootObservation $endBoot $observed (Read-RemovalUptime)
    if($endBoot -ne $boot -or -not $bootRecord.ClockConsistent -or ($observed-$started).TotalSeconds -gt 120){throw 'Boot changed or collection exceeded freshness bound.'}
    $snapshot=@{schema_version=1;host=$identity.host;sid=$identity.sid;session=$identity.session
        CollectorStartedUtc=$started.ToString('o');BootUtc=$bootRecord.BootUtc;ObservedUtc=$bootRecord.ObservedUtc
        BootAfterDeployment=$bootRecord.BootAfterDeployment;UptimeMilliseconds=$bootRecord.UptimeMilliseconds
        SignaturesVerified=$true;CatalogMembersVerified=$true;Driver=$driver
        NativeOwners=$processes.NativeOwners;ExternalOwners=$processes.ExternalOwners
        ExternalOwnersObservationStartedUtc=$processStarted.ToString('o')
        ExternalOwnersProvenance='ObservedComplete';ExternalOwnersObservedUtc=$bootRecord.ObservedUtc
        RunningTasks=$tasks;PresentRawChildren=$children;Defaults=$controls.Defaults;Render=$controls.Render;Security=$security
        NativeTestStarted=$false;RebootRequested=$false;Qualified=$false
        IdentityScope='Fresh postboot service-file/PnP provenance, not in-memory image attestation.'}
    return $snapshot
}
