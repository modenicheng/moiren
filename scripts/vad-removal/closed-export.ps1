#requires -Version 5.1
# Closed filesystem evidence only: no dispatch, transfer, deletion or release.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'finite-tool.ps1')
Add-Type -AssemblyName System.IO.Compression
if($null-eq(Get-Variable RemovalClosedExports -Scope Script -ErrorAction SilentlyContinue)){
    $script:RemovalClosedExports=New-Object 'Runtime.CompilerServices.ConditionalWeakTable[object,object]'
    $script:RemovalClosedExportAttempts=New-Object 'Runtime.CompilerServices.ConditionalWeakTable[object,object]'
}

function Get-RemovalClosedExportHash([byte[]]$Bytes){
    $sha=[Security.Cryptography.SHA256]::Create()
    try{[BitConverter]::ToString($sha.ComputeHash($Bytes)).Replace('-','')}finally{$sha.Dispose()}
}

function Get-RemovalClosedExportJson($Value){ConvertTo-Json -InputObject $Value -Depth 24 -Compress}

function Get-RemovalClosedExportSealJson($Seal){
    Get-RemovalClosedExportJson ([ordered]@{Directory=$Seal.Directory;Nonce=$Seal.Nonce
        ResultPin=$Seal.ResultPin;ResultJson=$Seal.ResultJson;ManifestPin=$Seal.ManifestPin
        InputJson=$Seal.InputJson;SourcePins=@($Seal.SourcePins);CollectorProvenance=$Seal.CollectorProvenance
        StagedScriptPins=@($Seal.StagedScriptPins);TerminalPins=@($Seal.TerminalPins)})
}

function Assert-RemovalClosedExportDestination([string]$Path,[string]$Directory,[string]$Extension){
    Assert-FiniteToolPath $Path $false
    $parent=[IO.Path]::GetDirectoryName($Directory)
    if(-not[String]::Equals([IO.Path]::GetDirectoryName($Path),$parent,[StringComparison]::OrdinalIgnoreCase)-or
        [IO.Path]::GetExtension($Path)-ine$Extension){throw 'Create-new sibling export path with exact extension required.'}
    if([IO.File]::Exists($Path)-or[IO.Directory]::Exists($Path)){throw 'Export output already exists; never overwrite or retry.'}
}

function Read-RemovalClosedExportFile([string]$Path,[switch]$Capture){
    Assert-FiniteToolPath $Path $false
    $file=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $sha=[Security.Cryptography.SHA256]::Create()
    try{
        if($file.Length-gt16777216){throw 'Export member exceeds 16 MiB cap.'}
        $length=$file.Length;$hash=[BitConverter]::ToString($sha.ComputeHash($file)).Replace('-','')
        $bytes=$null
        if($Capture){
            $file.Position=0;$copy=New-Object IO.MemoryStream
            try{$file.CopyTo($copy);$bytes=$copy.ToArray()}finally{$copy.Dispose()}
        }
        if($file.Length-ne$length){throw 'Export source changed during read.'}
        [pscustomobject]@{Path=$Path;Bytes=$length;Sha256=$hash;Content=$bytes}
    }finally{$sha.Dispose();$file.Dispose()}
}

function Get-RemovalClosedExportInventory($Seal){
    $root=$Seal.Directory;Assert-FiniteToolPath $root $false
    if(-not[IO.Directory]::Exists($root)){throw 'Original closed run directory missing.'}
    $prefix=$root+'\';$members=New-Object Collections.Generic.List[object]
    $pending=New-Object 'Collections.Generic.Stack[string]';$pending.Push($root)
    $seen=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $members.Add([pscustomobject][ordered]@{Name='run/';Kind='Directory';Bytes=[long]0;Sha256=$null})
    $total=[long]0
    $staged=@{}
    $stageNames=@('finite-tool.ps1','task-boundary.ps1','probe-boundary.ps1','worker-boundary.ps1','worker.ps1')
    foreach($pin in @($Seal.StagedScriptPins)){
        if($null-eq$pin){continue}
        $name=[IO.Path]::GetFileName($pin.Path)
        if($name-cnotin$stageNames-or$pin.Path-cne(Join-Path $root ('managed\'+$name))-or$staged.ContainsKey($pin.Path)){
            throw 'Only original five managed staged script paths are eligible.'
        }
        $staged[$pin.Path]=$pin
    }
    if($staged.Count-notin@(0,5)){throw 'Complete original five staged script pins required.'}
    while($pending.Count){
        $directory=$pending.Pop();Assert-FiniteToolPath $directory $false
        # Enumerate lazily and stop at the member cap, including hidden entries.
        foreach($path in [IO.Directory]::EnumerateFileSystemEntries($directory)){
            Assert-FiniteToolPath $path $false
            if(-not$path.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Export entry escaped original run.'}
            $relative=$path.Substring($prefix.Length).Replace('\','/')
            $parts=$relative.Split('/')
            if($parts.Count-gt8){throw 'Export depth exceeds eight run-relative components.'}
            foreach($part in $parts){
                if(-not$part-or$part-in@('.','..')-or$part-match'[<>:"|?*\x00-\x1f]'-or$part.TrimEnd(' ','.')-cne$part){
                    throw 'Noncanonical export member.'
                }
            }
            if(-not$seen.Add($relative)){throw 'Duplicate case-insensitive export member.'}
            # Reserve two members for immutable provenance and inventory.
            if($members.Count-ge510){throw 'Export exceeds 512 total members.'}
            $attributes=[IO.File]::GetAttributes($path)
            if($attributes-band[IO.FileAttributes]::ReparsePoint){throw 'Reparse export entry refused.'}
            if($attributes-band[IO.FileAttributes]::Directory){
                $members.Add([pscustomobject][ordered]@{Name=('run/'+$relative+'/');Kind='Directory';Bytes=[long]0;Sha256=$null})
                $pending.Push($path);continue
            }
            $extension=[IO.Path]::GetExtension($path).ToLowerInvariant()
            if($extension-ceq'.ps1'){
                if(-not$staged.ContainsKey($path)){throw 'Unpinned executable script in closed run.'}
            }elseif($extension-cnotin@('.json','.jsonl','.log','.txt','.sum','.etl','.xml','.csv','.wav','.raw','.pcm','.f32','.f32le','.u8','.u64le','.i64le','.bin','.out','.err','.stop')){
                throw 'Executable/container or unknown evidence artifact class refused; no silent omission.'
            }
            $actual=Read-RemovalClosedExportFile $path
            if($extension-ceq'.ps1'-and($actual.Bytes-ne$staged[$path].Bytes-or$actual.Sha256-ine$staged[$path].Sha256)){
                throw 'Original staged script pin changed.'
            }
            # Refuse executable images even when renamed to a data extension.
            $input=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
            try{$header=New-Object byte[] 4;$n=$input.Read($header,0,4)}finally{$input.Dispose()}
            $magic=[BitConverter]::ToString($header)
            if(($n-ge2-and$header[0]-eq77-and$header[1]-eq90)-or
                ($n-eq4-and$magic-in@('7F-45-4C-46','FE-ED-FA-CE','FE-ED-FA-CF','CE-FA-ED-FE','CF-FA-ED-FE','CA-FE-BA-BE'))){
                throw 'Executable image in evidence tree refused.'
            }
            $total+=$actual.Bytes
            if($total-gt67108864){throw 'Export exceeds 64 MiB total uncompressed cap.'}
            $members.Add([pscustomobject][ordered]@{Name=('run/'+$relative);Kind='File';Bytes=[long]$actual.Bytes;Sha256=$actual.Sha256})
        }
    }
    foreach($pin in (@($Seal.TerminalPins)+@($Seal.ResultPin)+@($Seal.ManifestPin)+@($Seal.StagedScriptPins))){
        if($null-eq$pin-or-not$pin.Path.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){continue}
        $name='run/'+$pin.Path.Substring($prefix.Length).Replace('\','/')
        $match=@($members|Where-Object{$_.Name-ceq$name-and$_.Kind-ceq'File'})
        if($match.Count-ne1-or$match[0].Bytes-ne$pin.Bytes-or$match[0].Sha256-ine$pin.Sha256){
            throw ('Original terminal/staged pin missing or changed: '+$pin.Path)
        }
    }
    if($Seal.ResultPin.Path-cne(Join-Path $root 'controller-result.json')){throw 'Original result must belong to this run.'}
    $resultBytes=[Text.UTF8Encoding]::new($false,$true).GetBytes($Seal.ResultJson)
    if($resultBytes.Length-ne$Seal.ResultPin.Bytes-or(Get-RemovalClosedExportHash $resultBytes)-ine$Seal.ResultPin.Sha256){
        throw 'Exact sealed result JSON differs from original result pin.'
    }
    return ,@($members|Sort-Object -Property Name)
}

function Write-RemovalClosedExportBytes([string]$Path,[byte[]]$Bytes){
    Assert-FiniteToolPath $Path $false
    $file=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$file.Write($Bytes,0,$Bytes.Length);$file.Flush($true)}finally{$file.Dispose()}
    $actual=Read-RemovalClosedExportFile $Path
    if($actual.Bytes-ne$Bytes.Length-or$actual.Sha256-cne(Get-RemovalClosedExportHash $Bytes)){throw 'Export sidecar readback differs.'}
    [pscustomobject][ordered]@{Path=$Path;Bytes=$actual.Bytes;Sha256=$actual.Sha256}
}

function Test-RemovalClosedExportArchive($State){
    Assert-FiniteToolPath $State.ArchivePath $false
    $file=[IO.File]::Open($State.ArchivePath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $sha=[Security.Cryptography.SHA256]::Create();$zip=$null
    try{
        if($file.Length-gt68157440){throw 'Export archive exceeds bounded container overhead cap.'}
        $archiveBytes=$file.Length;$archiveHash=[BitConverter]::ToString($sha.ComputeHash($file)).Replace('-','')
        $file.Position=0;$zip=[IO.Compression.ZipArchive]::new($file,[IO.Compression.ZipArchiveMode]::Read,$true)
        $expected=@{}
        foreach($member in $State.Members){$expected[$member.Name]=$member}
        if($zip.Entries.Count-ne$expected.Count){throw 'Independent archive membership readback differs.'}
        $seen=New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
        foreach($entry in $zip.Entries){
            if(-not$seen.Add($entry.FullName)-or-not$expected.ContainsKey($entry.FullName)-or
                $expected[$entry.FullName].Name-cne$entry.FullName){throw 'Foreign/duplicate/noncanonical archive entry.'}
            $member=$expected[$entry.FullName]
            if($entry.Length-ne$member.Bytes-or$entry.Length-gt16777216){throw 'Independent archive member length differs.'}
            $input=$entry.Open();$memberSha=[Security.Cryptography.SHA256]::Create()
            try{
                $buffer=New-Object byte[] 65536;$count=[long]0
                while(($read=$input.Read($buffer,0,$buffer.Length))-gt0){
                    $count+=$read
                    if($count-gt$member.Bytes){throw 'Archive inflated beyond exact bounded member.'}
                    $null=$memberSha.TransformBlock($buffer,0,$read,$buffer,0)
                }
                $null=$memberSha.TransformFinalBlock([byte[]]@(),0,0)
                $hash=[BitConverter]::ToString($memberSha.Hash).Replace('-','')
                if($count-ne$member.Bytes-or($member.Kind-cne'Directory'-and$hash-cne$member.Sha256)){
                    throw 'Independent archive member hash differs.'
                }
            }finally{$memberSha.Dispose();$input.Dispose()}
        }
        [pscustomobject][ordered]@{Path=$State.ArchivePath;Bytes=$archiveBytes;Sha256=$archiveHash}
    }finally{if($null-ne$zip){$zip.Dispose()};$sha.Dispose();$file.Dispose()}
}

function Export-RemovalClosedRun {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Controller,[Parameter(Mandatory=$true,Position=1)][string]$ArchivePath)
    $ErrorActionPreference='Stop'
    # Only the controller owns lifecycle sealing. A copied result/DTO cannot
    # replace this getter, and the export never observes or restarts live work.
    $seal=Get-RemovalClosedControllerSeal $Controller
    $prior=$null
    if($script:RemovalClosedExportAttempts.TryGetValue($Controller,[ref]$prior)){throw 'Original controller export already attempted; preserve outputs.'}
    $script:RemovalClosedExportAttempts.Add($Controller,[object]::new())
    $sealJson=Get-RemovalClosedExportSealJson $seal
    $seal=$sealJson|ConvertFrom-Json
    Assert-RemovalClosedExportDestination $ArchivePath $seal.Directory '.zip'
    $inventoryPath=$ArchivePath+'.inventory.json'
    Assert-RemovalClosedExportDestination $inventoryPath $seal.Directory '.json'
    $runMembers=Get-RemovalClosedExportInventory $seal
    $encoding=[Text.UTF8Encoding]::new($false,$true)
    $provenance=$encoding.GetBytes((Get-RemovalClosedExportJson ([ordered]@{SchemaVersion=1;Nonce=$seal.Nonce
        ResultPin=$seal.ResultPin;ResultJson=$seal.ResultJson;ManifestPin=$seal.ManifestPin;InputJson=$seal.InputJson
        SourcePins=@($seal.SourcePins);StagedScriptPins=@($seal.StagedScriptPins);TerminalPins=@($seal.TerminalPins)
        CollectorProvenance=$seal.CollectorProvenance;Transferred=$false;NativePassed=$false;Qualified=$false})))
    $provenanceMember=[pscustomobject][ordered]@{Name='export/provenance.json';Kind='File';Bytes=[long]$provenance.Length
        Sha256=(Get-RemovalClosedExportHash $provenance)}
    $inventory=$encoding.GetBytes((Get-RemovalClosedExportJson ([ordered]@{SchemaVersion=1;Nonce=$seal.Nonce;ResultPin=$seal.ResultPin
        Limits=[ordered]@{RunRelativeDepth=8;Members=512;FileBytes=16777216;TotalBytes=67108864}
        Members=@($runMembers)+@($provenanceMember)})))
    $inventoryMember=[pscustomobject][ordered]@{Name='export/inventory.json';Kind='File';Bytes=[long]$inventory.Length
        Sha256=(Get-RemovalClosedExportHash $inventory)}
    $members=@($runMembers)+@($provenanceMember,$inventoryMember)
    $total=[long]0
    foreach($member in $members){
        if($member.Bytes-gt16777216){throw 'Export metadata/member exceeds 16 MiB cap.'};$total+=$member.Bytes
    }
    if($members.Count-gt512-or$total-gt67108864){throw 'Complete export exceeds member/64 MiB budget; never truncate.'}
    $state=[pscustomobject]@{Controller=$Controller;SealJson=$sealJson;Seal=$seal;ArchivePath=$ArchivePath
        Members=$members;RunMembersJson=(Get-RemovalClosedExportJson $runMembers);ArchivePin=$null;InventoryPin=$null
        ProjectionJson=$null;AckAttempted=$false}
    $file=[IO.File]::Open($ArchivePath,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $zip=$null
    try{
        $zip=[IO.Compression.ZipArchive]::new($file,[IO.Compression.ZipArchiveMode]::Create,$true)
        foreach($member in $members){
            $entry=$zip.CreateEntry($member.Name,[IO.Compression.CompressionLevel]::NoCompression)
            if($member.Kind-ceq'Directory'){continue}
            $output=$entry.Open()
            try{
                if($member.Name-ceq'export/provenance.json'){$output.Write($provenance,0,$provenance.Length)}
                elseif($member.Name-ceq'export/inventory.json'){$output.Write($inventory,0,$inventory.Length)}
                else{
                    $path=Join-Path $seal.Directory $member.Name.Substring(4).Replace('/','\')
                    Assert-FiniteToolPath $path $false
                    $input=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
                    $hash=[Security.Cryptography.SHA256]::Create()
                    try{
                        if($input.Length-ne$member.Bytes){throw 'Source changed before archive copy.'}
                        $buffer=New-Object byte[] 65536;$copied=[long]0
                        while(($read=$input.Read($buffer,0,$buffer.Length))-gt0){
                            $copied+=$read
                            if($copied-gt$member.Bytes){throw 'Source grew during archive copy.'}
                            $null=$hash.TransformBlock($buffer,0,$read,$buffer,0);$output.Write($buffer,0,$read)
                        }
                        $null=$hash.TransformFinalBlock([byte[]]@(),0,0)
                        if($copied-ne$member.Bytes-or[BitConverter]::ToString($hash.Hash).Replace('-','')-cne$member.Sha256){
                            throw 'Source hash changed during locked archive copy.'
                        }
                    }finally{$hash.Dispose();$input.Dispose()}
                }
            }finally{$output.Dispose()}
        }
        $zip.Dispose();$zip=$null;$file.Flush($true)
    }finally{if($null-ne$zip){$zip.Dispose()};$file.Dispose()}
    $state.InventoryPin=Write-RemovalClosedExportBytes $inventoryPath $inventory
    $state.ArchivePin=Test-RemovalClosedExportArchive $state
    if((Get-RemovalClosedExportJson (Get-RemovalClosedExportInventory $seal))-cne$state.RunMembersJson){throw 'Run membership/content changed during export.'}
    if((Get-RemovalClosedExportSealJson (Get-RemovalClosedControllerSeal $Controller))-cne$sealJson){throw 'Original controller terminal seal changed.'}
    $projection=[pscustomobject][ordered]@{SchemaVersion=1;Nonce=$seal.Nonce;ResultPin=$seal.ResultPin
        ArchivePin=$state.ArchivePin;InventoryPin=$state.InventoryPin;LocalVerified=$true;Transferred=$false;NativePassed=$false;Qualified=$false}
    $state.ProjectionJson=Get-RemovalClosedExportJson $projection
    # Public pins are a projection, not aliases into the retained export state.
    $projection=$state.ProjectionJson|ConvertFrom-Json
    $script:RemovalClosedExports.Add($projection,$state)
    return $projection
}

function Confirm-RemovalClosedRunExport {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true,Position=0)]$Export,[Parameter(Mandatory=$true,Position=1)][string]$AcknowledgementPath)
    $ErrorActionPreference='Stop';$state=$null
    if(-not$script:RemovalClosedExports.TryGetValue($Export,[ref]$state)){throw 'Original retained closed export required; detached projection is not authority.'}
    if($state.AckAttempted){throw 'Original export acknowledgement already attempted; no retry or overwrite.'}
    $state.AckAttempted=$true
    if((Get-RemovalClosedExportJson $Export)-cne$state.ProjectionJson){throw 'Original export projection changed.'}
    if((Get-RemovalClosedExportSealJson (Get-RemovalClosedControllerSeal $state.Controller))-cne$state.SealJson){throw 'Original controller terminal seal changed.'}
    Assert-RemovalClosedExportDestination $AcknowledgementPath $state.Seal.Directory '.json'
    $inventory=Read-RemovalClosedExportFile $state.InventoryPin.Path
    if($inventory.Bytes-ne$state.InventoryPin.Bytes-or$inventory.Sha256-cne$state.InventoryPin.Sha256){throw 'Original inventory sidecar changed.'}
    $archive=Test-RemovalClosedExportArchive $state
    if($archive.Bytes-ne$state.ArchivePin.Bytes-or$archive.Sha256-cne$state.ArchivePin.Sha256){throw 'Original archive bytes changed.'}
    if((Get-RemovalClosedExportJson (Get-RemovalClosedExportInventory $state.Seal))-cne$state.RunMembersJson){throw 'Original run evidence changed before local acknowledgement.'}
    $value=[ordered]@{SchemaVersion=1;Meaning='LOCAL_ARCHIVE_VERIFICATION_ONLY';Nonce=$state.Seal.Nonce
        ResultPin=$state.Seal.ResultPin;InventoryPin=$state.InventoryPin;ArchivePin=$state.ArchivePin
        Transferred=$false;NativePassed=$false;Qualified=$false}
    $bytes=[Text.UTF8Encoding]::new($false,$true).GetBytes((Get-RemovalClosedExportJson $value))
    return Write-RemovalClosedExportBytes $AcknowledgementPath $bytes
}
