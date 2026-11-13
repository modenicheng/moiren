#requires -Version 5.1
# File-only transport helper, independently pinned outside the native source set.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$RequestPath)
Set-StrictMode -Version 2
$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if($env:COMPUTERNAME-cne'LAPTOP-NP49RG6H'-or
    [Security.Principal.WindowsIdentity]::GetCurrent().User.Value-cne'S-1-5-21-3080296863-1242338503-1346362208-1001'){
    throw 'Wrong file handoff host/SID'
}
$locks=New-Object 'Collections.Generic.List[object]'

function Assert-HandoffPath([string]$Path,[switch]$SystemExecutable){
    if($Path.Length-gt1024-or$Path-cnotmatch'\A[A-Z]:\\'-or[IO.Path]::GetFullPath($Path)-cne$Path-or
        $Path.Substring(2).Contains(':')-or$Path-match'[\x00-\x1f\x7f"*?<>|/]'-or
        (-not$SystemExecutable-and-not$Path.StartsWith('D:\MoirenTest\',[StringComparison]::Ordinal))){
        throw 'Canonical literal handoff path required'
    }
    foreach($part in $Path.Substring(3).Split('\')){
        if(-not$part-or$part-ceq'.'-or$part-ceq'..'-or$part.EndsWith('.')-or$part.EndsWith(' ')-or
            $part-match'\A(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\.|\z)'){throw 'Reserved handoff path component'}
    }
    $cursor=$Path
    while($cursor){
        if(([IO.File]::Exists($cursor)-or[IO.Directory]::Exists($cursor))-and
            (([IO.File]::GetAttributes($cursor)-band[IO.FileAttributes]::ReparsePoint)-ne0)){throw 'Reparse handoff path'}
        $cursor=[IO.Path]::GetDirectoryName($cursor)
    }
}

function Assert-HandoffFields($Value,[string[]]$Names){
    if($Value-isnot[pscustomobject]){throw 'Typed handoff object required'}
    $actual=@($Value.PSObject.Properties.Name)
    if($actual.Count-ne$Names.Count){throw 'Exact handoff fields required'}
    foreach($name in $Names){if(-not($actual-ccontains$name)){throw 'Missing exact handoff field'}}
}

function Read-HandoffJson([string]$Text){
    if($Text-cnotmatch'\A\s*\{'){throw 'Strict handoff JSON object required'}
    $pattern='\G\s*(?:(?<String>"(?:[^"\\\x00-\x1f]|\\(?:["\\/bfnrt]|u[0-9A-Fa-f]{4}))*")|(?<Number>-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)|(?<Literal>true|false|null)|(?<Punct>[{}\[\],:]))'
    $expression=[regex]::new($pattern);$stack=New-Object 'Collections.Generic.List[object]'
    $position=0;$tokens=0
    while($position-lt$Text.Length){
        $match=$expression.Match($Text,$position)
        if(-not$match.Success-or$match.Index-ne$position){
            if([string]::IsNullOrWhiteSpace($Text.Substring($position))){break}
            throw 'Strict handoff JSON syntax required'
        }
        $position+=$match.Length;$tokens++
        if($tokens-gt50000){throw 'Handoff JSON token cap exceeded'}
        if($match.Groups['String'].Success-and$stack.Count-gt0){
            $top=$stack[$stack.Count-1]
            if($top.Kind-ceq'Object'-and$top.NeedKey){
                $key=(ConvertFrom-Json -InputObject ('{"K":'+$match.Groups['String'].Value+'}')).K
                if($top.Keys.ContainsKey($key)){throw 'Duplicate/case-ambiguous handoff JSON key'}
                $top.Keys[$key]=$true;$top.NeedKey=$false
            }
        }elseif($match.Groups['Number'].Success){
            $number=[double]0
            if(-not[double]::TryParse($match.Groups['Number'].Value,[Globalization.NumberStyles]::Float,
                [Globalization.CultureInfo]::InvariantCulture,[ref]$number)-or
                [double]::IsNaN($number)-or[double]::IsInfinity($number)){throw 'Finite handoff numbers required'}
        }elseif($match.Groups['Punct'].Success){
            $p=$match.Groups['Punct'].Value
            if($p-ceq'{'-or$p-ceq'['){
                if($stack.Count-ge32){throw 'Handoff JSON depth cap exceeded'}
                $kind=if($p-ceq'{'){'Object'}else{'Array'}
                $stack.Add(@{Kind=$kind;NeedKey=($kind-ceq'Object');Keys=@{}})
            }elseif($p-ceq'}'-or$p-ceq']'){
                if($stack.Count-eq0-or(($p-ceq'}')-ne($stack[$stack.Count-1].Kind-ceq'Object'))){throw 'Unbalanced handoff JSON'}
                $stack.RemoveAt($stack.Count-1)
            }elseif($p-ceq','-and$stack.Count-gt0-and$stack[$stack.Count-1].Kind-ceq'Object'){
                $stack[$stack.Count-1].NeedKey=$true
            }
        }
    }
    if($stack.Count-ne0){throw 'Incomplete handoff JSON'}
    $value=ConvertFrom-Json -InputObject $Text
    if($value-isnot[pscustomobject]){throw 'Handoff object required'}
    return $value
}

function Open-HandoffFile([string]$Path,[long]$Cap,$Pin=$null,[switch]$SystemExecutable){
    Assert-HandoffPath $Path -SystemExecutable:$SystemExecutable
    $s=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $locks.Add($s)
    if($s.Length-lt0-or$s.Length-gt$Cap){throw 'Handoff input cap exceeded'}
    if($null-ne$Pin){
        Assert-HandoffFields $Pin @('Path','Bytes','Sha256')
        if($Pin.Path-cne$Path-or($Pin.Bytes-isnot[int]-and$Pin.Bytes-isnot[long])-or
            $Pin.Bytes-ne$s.Length-or$Pin.Sha256-isnot[string]-or$Pin.Sha256-cnotmatch'\A[A-Fa-f0-9]{64}\z'){
            throw 'Exact handoff input pin required'
        }
        $h=[Security.Cryptography.SHA256]::Create()
        try{$sha=[BitConverter]::ToString($h.ComputeHash($s)).Replace('-','')}finally{$h.Dispose()}
        if($sha-ine$Pin.Sha256){throw 'Handoff input SHA256 differs'}
        $s.Position=0
    }
    return ,$s
}

function Read-HandoffText($Stream){
    $Stream.Position=0;$bytes=New-Object byte[] $Stream.Length;$offset=0
    while($offset-lt$bytes.Length){
        $n=$Stream.Read($bytes,$offset,$bytes.Length-$offset)
        if($n-le0){throw 'Short original handoff file'}
        $offset+=$n
    }
    return [Text.UTF8Encoding]::new($false,$true).GetString($bytes)
}

try{
    $requestStream=Open-HandoffFile $RequestPath 1048576
    $request=Read-HandoffJson (Read-HandoffText $requestStream)
    Assert-HandoffFields $request @('SchemaVersion','Operation','Arguments','PayloadPin')
    if(($request.SchemaVersion-isnot[int]-and$request.SchemaVersion-isnot[long])-or$request.SchemaVersion-ne1){throw 'Handoff schema1 required'}
    $a=$request.Arguments
    switch -CaseSensitive ($request.Operation){
        'Download'{
            Assert-HandoffFields $a @('RemotePath','Cap')
            if($null-ne$request.PayloadPin-or($a.Cap-isnot[int]-and$a.Cap-isnot[long])-or$a.Cap-lt1-or$a.Cap-gt68157440){throw 'Exact bounded download required'}
            $s=Open-HandoffFile $a.RemotePath $a.Cap
            $length=$s.Length;$out=[Console]::OpenStandardOutput();$buffer=New-Object byte[] 65536;$total=[long]0
            while(($n=$s.Read($buffer,0,$buffer.Length))-gt0){
                $total+=$n
                if($total-gt$a.Cap-or$total-gt$length){throw 'Download input exceeded original bound'}
                $out.Write($buffer,0,$n)
            }
            if($total-ne$length-or$s.Length-ne$length){throw 'Download input became short/changed'}
            $out.Flush()
        }
        'Entry'{
            if($null-ne$request.PayloadPin){throw 'Entry has no separate payload'}
            Assert-HandoffFields $a @('SchemaVersion','RunDirectory','ManifestPin','SourceDirectory','SourcesRevision','Sources','PowerShellPin','WorkerSessionId')
            if(-not([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole(
                [Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Already elevated entry required; no automatic elevation'}
            Assert-HandoffPath $a.RunDirectory;Assert-HandoffPath $a.SourceDirectory
            if(Test-Path -LiteralPath $a.RunDirectory){throw 'Create-new diagnostic run required'}
            $manifestFile=Open-HandoffFile $a.ManifestPin.Path 1048576 $a.ManifestPin
            $manifest=Read-HandoffJson (Read-HandoffText $manifestFile)
            if($manifest.SourcesRevision-cne$a.SourcesRevision-or$manifest.Sources.Count-ne$a.Sources.Count){throw 'Original manifest/source binding differs'}
            $seen=@{};$entry=$null
            foreach($pin in $a.Sources){
                Assert-HandoffFields $pin @('Name','Path','Bytes','Sha256')
                if($pin.Name-isnot[string]-or$pin.Name-cnotmatch'\A[a-z][a-z0-9-]*\.ps1\z'-or$seen.ContainsKey($pin.Name)-or
                    $pin.Path-cne(Join-Path $a.SourceDirectory $pin.Name)){throw 'Duplicate/outside source row'}
                $seen[$pin.Name]=$true
                $matches=@($manifest.Sources|Where-Object{$_.Name-ceq$pin.Name-and$_.Path-ceq$pin.Path-and$_.Bytes-eq$pin.Bytes-and$_.Sha256-ceq$pin.Sha256})
                if($matches.Count-ne1){throw 'Original source pin differs'}
                $null=Open-HandoffFile $pin.Path 1048576 ([pscustomobject]@{Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256})
                if($pin.Name-ceq'entry-launch.ps1'){$entry=$pin.Path}
            }
            $actual=@(Get-ChildItem -LiteralPath $a.SourceDirectory -Force -Filter '*.ps1'|Where-Object{$_.Name-notlike'test-*'})
            if($actual.Count-ne$a.Sources.Count-or$null-eq$entry){throw 'Complete original source set required'}
            foreach($f in $actual){if($f.PSIsContainer-or-not$seen.ContainsKey($f.Name)){throw 'Unpinned production source'}}
            if([IO.Path]::GetFileName($a.PowerShellPin.Path)-cne'powershell.exe'){throw 'Pinned Windows PowerShell required'}
            $null=Open-HandoffFile $a.PowerShellPin.Path 268435456 $a.PowerShellPin -SystemExecutable
            . $entry
            $result=Invoke-RemovalDiagnosticEntry -RunDirectory $a.RunDirectory -ManifestPath $a.ManifestPin.Path `
                -ManifestSha256 $a.ManifestPin.Sha256 -SourceDirectory $a.SourceDirectory `
                -PowerShellPin $a.PowerShellPin -UserRebootConfirmed -WorkerSessionId $a.WorkerSessionId
            [Console]::Write(($result|ConvertTo-Json -Depth 12 -Compress))
        }
        'Finalize'{
            Assert-HandoffFields $a @('RemoteRun','LocalAckSha256','FinalizerPath','FinalizerSha256','DependencyPins')
            Assert-HandoffPath $a.RemoteRun;Assert-HandoffPath $a.FinalizerPath
            if(-not[IO.Directory]::Exists($a.RemoteRun)-or[IO.Path]::GetFileName($a.FinalizerPath)-cne'delivery-finalize.ps1'-or
                $a.LocalAckSha256-cnotmatch'\A[A-Fa-f0-9]{64}\z'-or$a.FinalizerSha256-cnotmatch'\A[A-Fa-f0-9]{64}\z'){
                throw 'Original finalize identity required'
            }
            $s=Open-HandoffFile $request.PayloadPin.Path 1048576 $request.PayloadPin
            $payload=Read-HandoffText $s;$null=Read-HandoffJson $payload
            $final=Open-HandoffFile $a.FinalizerPath 1048576
            $h=[Security.Cryptography.SHA256]::Create()
            try{$sha=[BitConverter]::ToString($h.ComputeHash($final)).Replace('-','')}finally{$h.Dispose()}
            if($sha-ine$a.FinalizerSha256){throw 'Finalizer SHA256 differs'}
            $expected=@{};foreach($name in @('closed-export.ps1','finite-tool.ps1')){$expected[(Join-Path ([IO.Path]::GetDirectoryName($a.FinalizerPath)) $name)]=$true}
            if($a.DependencyPins-isnot[array]-or$a.DependencyPins.Count-ne2){throw 'Both exact finalizer dependencies required'}
            foreach($pin in $a.DependencyPins){
                if(-not$expected.ContainsKey($pin.Path)){throw 'Foreign/duplicate finalizer dependency'}
                $expected.Remove($pin.Path);$null=Open-HandoffFile $pin.Path 1048576 $pin
            }
            $upload=$a.RemoteRun+'.delivery-host.json';Assert-HandoffPath $upload
            $s.Position=0;$out=[IO.File]::Open($upload,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
            try{$s.CopyTo($out);$out.Flush($true)}finally{$out.Dispose()}
            $null=Open-HandoffFile $upload 1048576 ([pscustomobject]@{Path=$upload;Bytes=$request.PayloadPin.Bytes;Sha256=$request.PayloadPin.Sha256})
            . $a.FinalizerPath
            $result=Confirm-RemovalRunDelivery -RunDirectory $a.RemoteRun -DeliveryReceiptPath $upload `
                -ExpectedDeliveryReceiptSha256 $request.PayloadPin.Sha256 -ExpectedLocalAckSha256 $a.LocalAckSha256 -UserRebootConfirmed
            [Console]::Write(($result|ConvertTo-Json -Depth 16 -Compress))
        }
        default{throw 'Unknown file handoff operation'}
    }
}finally{foreach($s in $locks){$s.Dispose()}}
