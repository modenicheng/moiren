#requires -Version 5.1
# Closed-file manifest construction only. The independently reviewed layout
# binds source revision to exact source rows and original release/tool/TMF pins;
# this module does not discover a revision, rebuild, sign or launch anything.
Set-StrictMode -Version 2
. (Join-Path $PSScriptRoot 'run-inputs.ps1')

function Assert-RemovalManifestFields($Value,[string[]]$Names){
    if($null-eq$Value-or$Value-isnot[pscustomobject]){throw 'Typed layout object required.'}
    $actual=@($Value.PSObject.Properties.Name)
    if($actual.Count-ne$Names.Count){throw 'Layout fields differ from the explicit schema.'}
    foreach($name in $Names){if(-not($actual-ccontains$name)){throw ('Missing exact layout field: '+$name)}}
}

function Assert-RemovalManifestPath([string]$Path){
    Assert-FiniteToolPath $Path $false
    if($Path-match'[\x00-\x1f\x7f<>|]' -or $Path.Length-gt1024){throw 'Bounded literal manifest/input path required.'}
    foreach($part in $Path.Substring(3).Split('\')){
        if(-not$part-or$part.EndsWith('.')-or$part.EndsWith(' ')-or
            $part-match'\A(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\.|\z)'){
            throw 'Noncanonical/reserved Windows path component.'
        }
    }
}

function ConvertFrom-RemovalManifestJson([string]$Text){
    # ConvertFrom-Json differs between PS5/PS7 for duplicate/case-ambiguous keys
    # and nonfinite numbers. Reject those before using its ordinary object model.
    if($Text-cnotmatch'\A\s*\{'){throw 'Layout JSON root must be an object.'}
    $pattern='\G\s*(?:(?<String>"(?:[^"\\\x00-\x1f]|\\(?:["\\/bfnrt]|u[0-9A-Fa-f]{4}))*")|(?<Number>-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)|(?<Literal>true|false|null)|(?<Punct>[{}\[\],:]))'
    $expression=[regex]::new($pattern)
    $stack=New-Object Collections.Generic.List[object]
    $position=0;$tokens=0
    while($position-lt$Text.Length){
        $match=$expression.Match($Text,$position)
        if(-not$match.Success-or$match.Index-ne$position){
            if([string]::IsNullOrWhiteSpace($Text.Substring($position))){break}
            throw 'Strict bounded JSON syntax required.'
        }
        $position+=$match.Length;$tokens++
        if($tokens-gt50000){throw 'Layout JSON token cap exceeded.'}
        if($match.Groups['String'].Success-and$stack.Count-gt0){
            $top=$stack[$stack.Count-1]
            if($top.Kind-ceq'Object'-and$top.NeedKey){
                $key=(ConvertFrom-Json -InputObject ('{"K":'+$match.Groups['String'].Value+'}') -ErrorAction Stop).K
                if($top.Keys.ContainsKey($key)){throw 'Duplicate/case-ambiguous layout JSON property.'}
                $top.Keys[$key]=$true;$top.NeedKey=$false
            }
        }elseif($match.Groups['Number'].Success){
            $number=[double]0
            if(-not[double]::TryParse($match.Groups['Number'].Value,[Globalization.NumberStyles]::Float,
                [Globalization.CultureInfo]::InvariantCulture,[ref]$number)-or
                [double]::IsInfinity($number)-or[double]::IsNaN($number)){throw 'Finite layout JSON numbers required.'}
        }elseif($match.Groups['Punct'].Success){
            $punct=$match.Groups['Punct'].Value
            if($punct-ceq'{'-or$punct-ceq'['){
                if($stack.Count-ge32){throw 'Layout JSON depth cap exceeded.'}
                $kind=if($punct-ceq'{'){'Object'}else{'Array'}
                $stack.Add(@{Kind=$kind;NeedKey=($kind-ceq'Object');Keys=@{}})
            }elseif($punct-ceq'}'-or$punct-ceq']'){
                if($stack.Count-eq0){throw 'Unbalanced layout JSON.'}
                $top=$stack[$stack.Count-1]
                if(($punct-ceq'}')-ne($top.Kind-ceq'Object')){throw 'Unbalanced layout JSON container.'}
                $stack.RemoveAt($stack.Count-1)
            }elseif($punct-ceq','-and$stack.Count-gt0){
                $top=$stack[$stack.Count-1]
                if($top.Kind-ceq'Object'){$top.NeedKey=$true}
            }
        }
    }
    if($stack.Count-ne0){throw 'Incomplete layout JSON.'}
    $value=ConvertFrom-Json -InputObject $Text -ErrorAction Stop
    if($value-isnot[pscustomobject]){throw 'Layout JSON root must be an object.'}
    return $value
}

function Read-RemovalManifestPinnedFile($Pin,[long]$MaximumBytes,$Handles){
    Assert-RemovalManifestFields $Pin @('Path','Bytes','Sha256')
    if($Pin.Path-isnot[string]-or$Pin.Sha256-isnot[string]-or$Pin.Sha256-cnotmatch'\A[A-Fa-f0-9]{64}\z'){
        throw 'Expected literal path and independent SHA256 pin required.'
    }
    Assert-RemovalRunInteger $Pin.Bytes 1 $MaximumBytes
    Assert-RemovalManifestPath $Pin.Path
    $stream=[IO.File]::Open($Pin.Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    $Handles.Add($stream)
    if($stream.Length-ne$Pin.Bytes){throw 'Reviewed input file length differs.'}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$actual=[BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-','')}
    finally{$sha.Dispose()}
    if($actual-ine$Pin.Sha256-or$stream.Length-ne$Pin.Bytes){throw 'Reviewed input file content differs; no adoption.'}
    [pscustomobject][ordered]@{Path=$Pin.Path;Bytes=[long]$stream.Length;Sha256=$actual}
}

function Read-RemovalManifestSourceProvenance($Value,$Sources,[string]$Revision,$Handles){
    Assert-RemovalManifestFields $Value @('SourceKind','BaseCommit','BundleIdentity','SourcesRevisionMeaning','BundleManifestPin')
    if($Value.SourceKind-cne'ReviewedBundle'-or$Value.BaseCommit-isnot[string]-or
        $Value.BaseCommit-cnotmatch'\A[0-9a-f]{40}\z'-or$Value.BaseCommit-cne$Revision-or
        $Value.SourcesRevisionMeaning-cne'ReviewedBundleBaseCommit'-or$Value.BundleIdentity-isnot[string]-or
        $Value.BundleIdentity-cnotmatch'\A[A-Za-z0-9][A-Za-z0-9._-]{0,127}\z'){
        throw 'Honest independently reviewed bundle identity/base required; GitTree is not implemented.'
    }
    $pin=Read-RemovalManifestPinnedFile $Value.BundleManifestPin 1048576 $Handles
    $file=Read-RemovalDecodeFile $pin.Path 1048576 -Text
    $bundle=ConvertFrom-RemovalManifestJson $file.Text
    Assert-RemovalManifestFields $bundle @('SchemaVersion','SourceKind','BaseCommit','BundleIdentity','Sources')
    Assert-RemovalRunInteger $bundle.SchemaVersion 1 1
    if($bundle.SourceKind-cne$Value.SourceKind-or$bundle.BaseCommit-cne$Value.BaseCommit-or
        $bundle.BundleIdentity-cne$Value.BundleIdentity-or$bundle.Sources-isnot[array]-or
        $bundle.Sources.Count-ne$Sources.Count){throw 'Independent source bundle identity/membership differs.'}
    $seen=@{}
    foreach($row in $bundle.Sources){
        Assert-RemovalManifestFields $row @('Name','Bytes','Sha256')
        Assert-RemovalRunInteger $row.Bytes 1 1048576
        if($row.Name-isnot[string]-or$seen.ContainsKey($row.Name)-or$row.Sha256-isnot[string]-or
            $row.Sha256-cnotmatch'\A[A-Fa-f0-9]{64}\z'){throw 'Unique exact reviewed source bundle row required.'}
        $seen[$row.Name]=$true
        $actual=@($Sources|Where-Object{$_.Name-ceq$row.Name})
        if($actual.Count-ne1-or$actual[0].Bytes-ne$row.Bytes-or$actual[0].Sha256-ine$row.Sha256){
            throw 'Actual source content differs from independent reviewed bundle; paths may relocate, content may not.'
        }
    }
    [pscustomobject][ordered]@{SourceKind='ReviewedBundle';BaseCommit=$Value.BaseCommit
        BundleIdentity=$Value.BundleIdentity;SourcesRevisionMeaning='ReviewedBundleBaseCommit';BundleManifestPin=$pin}
}

function New-RemovalRunManifest {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$LayoutPath,
        [Parameter(Mandatory=$true)][ValidatePattern('\A[A-Fa-f0-9]{64}\z')][string]$ExpectedLayoutSha256,
        [Parameter(Mandatory=$true)][string]$ManifestPath)
    $ErrorActionPreference='Stop'
    Assert-RemovalManifestPath $LayoutPath;Assert-RemovalManifestPath $ManifestPath
    if(Test-Path -LiteralPath $ManifestPath){throw 'Manifest publication is create-new only; existing artifacts are retained.'}
    if(-not[IO.Directory]::Exists([IO.Path]::GetDirectoryName($ManifestPath))){throw 'Existing canonical manifest parent directory required.'}
    $handles=New-Object Collections.Generic.List[object]
    try{
        $layoutHandle=[IO.File]::Open($LayoutPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        $handles.Add($layoutHandle)
        $layoutFile=Read-RemovalDecodeFile $LayoutPath 1048576 -Text
        if($layoutFile.Sha256-ine$ExpectedLayoutSha256){throw 'Independent reviewed layout SHA256 differs.'}
        $layout=ConvertFrom-RemovalManifestJson $layoutFile.Text
        Assert-RemovalManifestFields $layout @('SchemaVersion','SourcesRevision','SourceProvenance','SourceDirectory','Sources','Probe','Driver','Schedule','Tools','Symbols')
        Assert-RemovalRunInteger $layout.SchemaVersion 1 1
        if($layout.SourcesRevision-isnot[string]-or$layout.SourcesRevision-cnotmatch'\A[0-9a-f]{40}\z'-or
            $layout.SourceDirectory-isnot[string]){throw 'Reviewed source revision and exact source directory required.'}
        Assert-RemovalManifestPath $layout.SourceDirectory
        if($layout.Sources-isnot[array]-or$layout.Sources.Count-lt28-or$layout.Sources.Count-gt64){throw 'Complete reviewed production source row set required.'}
        $sources=@()
        foreach($row in $layout.Sources){
            Assert-RemovalManifestFields $row @('Name','Path','Bytes','Sha256')
            $pin=Read-RemovalManifestPinnedFile ([pscustomobject]@{Path=$row.Path;Bytes=$row.Bytes;Sha256=$row.Sha256}) 1048576 $handles
            $sources+=,[pscustomobject][ordered]@{Name=$row.Name;Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256}
        }
        if(@($sources|Where-Object{$_.Name-ceq'run-manifest.ps1'}).Count-ne1){throw 'The reviewed builder itself must belong to the production source set.'}
        $sourceProvenance=Read-RemovalManifestSourceProvenance $layout.SourceProvenance $sources $layout.SourcesRevision $handles
        Assert-RemovalManifestFields $layout.Probe @('File','BuildManifest','SourceRevision')
        $probe=[pscustomobject][ordered]@{
            File=(Read-RemovalManifestPinnedFile $layout.Probe.File 268435456 $handles)
            BuildManifest=(Read-RemovalManifestPinnedFile $layout.Probe.BuildManifest 1048576 $handles)
            SourceRevision=$layout.Probe.SourceRevision}
        Assert-RemovalManifestFields $layout.Driver @('PackageManifest','Files','Pdb')
        if($layout.Driver.Files-isnot[array]-or$layout.Driver.Files.Count-ne3){throw 'Exact original signed package membership required.'}
        $driverFiles=@()
        foreach($row in $layout.Driver.Files){
            Assert-RemovalManifestFields $row @('Name','Path','Bytes','Sha256')
            $pin=Read-RemovalManifestPinnedFile ([pscustomobject]@{Path=$row.Path;Bytes=$row.Bytes;Sha256=$row.Sha256}) 16777216 $handles
            $driverFiles+=,[pscustomobject][ordered]@{Name=$row.Name;Path=$pin.Path;Bytes=$pin.Bytes;Sha256=$pin.Sha256}
        }
        $driver=[pscustomobject][ordered]@{
            PackageManifest=(Read-RemovalManifestPinnedFile $layout.Driver.PackageManifest 1048576 $handles)
            Files=$driverFiles;Pdb=(Read-RemovalManifestPinnedFile $layout.Driver.Pdb 16777216 $handles)}
        $schedule=Read-RemovalManifestPinnedFile $layout.Schedule 1048576 $handles
        Assert-RemovalManifestFields $layout.Tools @('Tracefmt','Python','Reader','SignTool','Catalog','BcdEdit')
        $tools=[ordered]@{}
        foreach($entry in @(@('Tracefmt',268435456),@('Python',268435456),@('Reader',1048576),
            @('SignTool',268435456),@('Catalog',268435456),@('BcdEdit',268435456))){
            $tools[$entry[0]]=Read-RemovalManifestPinnedFile $layout.Tools.($entry[0]) $entry[1] $handles
        }
        Assert-RemovalManifestFields $layout.Symbols @('Directory','Files','DriverRevision','PdbSha256')
        Assert-RemovalManifestPath $layout.Symbols.Directory
        $fixed=Get-RemovalRunFixedPins
        if($layout.Symbols.DriverRevision-isnot[string]-or$layout.Symbols.DriverRevision-cne$fixed.DriverRevision-or
            $layout.Symbols.PdbSha256-isnot[string]-or$layout.Symbols.PdbSha256-ine$fixed.PdbSha256-or
            $layout.Symbols.Files-isnot[array]-or$layout.Symbols.Files.Count-lt1-or$layout.Symbols.Files.Count-gt64){
            throw 'Independent original613 PDB/revision and exact TMF membership pins required.'
        }
        $symbols=@()
        foreach($row in $layout.Symbols.Files){$symbols+=,(Read-RemovalManifestPinnedFile $row 1048576 $handles)}
        $manifest=[pscustomobject][ordered]@{SchemaVersion=1;SourcesRevision=$layout.SourcesRevision
            SourceProvenance=$sourceProvenance;Sources=$sources
            Probe=$probe;Driver=$driver;Schedule=$schedule;Tools=[pscustomobject]$tools
            Symbols=[pscustomobject][ordered]@{Directory=$layout.Symbols.Directory;Files=$symbols
                DriverRevision=$layout.Symbols.DriverRevision;PdbSha256=$layout.Symbols.PdbSha256}}
        # Use the real consumer before CREATE_NEW and again against the durable
        # bytes. Expected constants, source membership and legacy schema stay in
        # one authority; the builder never changes or mocks them in production.
        $null=Read-RemovalRunPins $manifest $layout.SourceDirectory
        $text=ConvertTo-Json -InputObject $manifest -Depth 16 -Compress
        $bytes=[Text.UTF8Encoding]::new($false,$true).GetBytes($text)
        if($bytes.Length-gt1048576){throw 'Generated manifest exceeds original consumer cap.'}
        $sha=[Security.Cryptography.SHA256]::Create()
        try{$expected=[BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-','')}
        finally{$sha.Dispose()}
        Assert-RemovalManifestPath $ManifestPath
        $output=[IO.File]::Open($ManifestPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::Read)
        $handles.Add($output)
        $output.Write($bytes,0,$bytes.Length);$output.Flush($true)
        # Windows share checks are symmetric: the consumer's FileShare.Read
        # cannot coexist with our write-access handle. Close only that writer,
        # reopen a retained read-only handle, then independently check all bytes.
        # Any replacement in this transition must still match the expected hash.
        $output.Dispose();Assert-RemovalManifestPath $ManifestPath
        $published=[IO.File]::Open($ManifestPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        $handles.Add($published)
        $actual=Read-RemovalDecodeFile $ManifestPath 1048576 -Text
        if($actual.Bytes-ne$bytes.Length-or$actual.Sha256-cne$expected-or$actual.Text-cne$text){throw 'Durable manifest readback differs.'}
        $owner=New-RemovalRunInputs -ManifestPath $ManifestPath -ExpectedSha256 $expected -SourceDirectory $layout.SourceDirectory
        Assert-RemovalRunInputs $owner
        $last=Read-RemovalDecodeFile $LayoutPath 1048576 -Text
        if($last.Sha256-ine$ExpectedLayoutSha256-or$last.Bytes-ne$layoutFile.Bytes-or$last.Text-cne$layoutFile.Text){throw 'Original reviewed layout changed during publication.'}
        return [pscustomobject][ordered]@{Path=$ManifestPath;Bytes=[long]$bytes.Length;Sha256=$expected}
    }finally{
        # Retain all input/output handles until the complete consumer readback.
        # Failure leaves CREATE_NEW artifacts for diagnosis, never deletes them.
        for($i=$handles.Count-1;$i-ge0;$i--){$handles[$i].Dispose()}
    }
}
