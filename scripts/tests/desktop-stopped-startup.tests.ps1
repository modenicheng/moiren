param([string]$Helper=(Join-Path $PSScriptRoot '..\desktop-stopped-startup.ps1'))
$ErrorActionPreference='Stop';Set-StrictMode -Version 2
function Require($ok,[string]$message){if(-not$ok){throw ('ASSERTION: '+$message)}}
Require (Test-Path -LiteralPath $Helper -PathType Leaf) 'Stopped-startup helper is missing'
. $Helper
$testRoot=Join-Path ([IO.Path]::GetTempPath()) ('moiren-stopped-startup-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot|Out-Null
$passed=0
$junctions=@()
function Case([string]$name,[scriptblock]$body){& $body;$script:passed++;Write-Output ('PASS '+$name)}
function Reject([scriptblock]$body,[string]$message){$rejected=$false;try{& $body|Out-Null}catch{$rejected=$true};Require $rejected $message}
function Fixture([string]$name){$run=Join-Path $testRoot $name;New-Item -ItemType Directory -Path $run|Out-Null;New-MoirenStoppedUiStartup -RunDirectory $run}
function Set-Json([string]$path,$value){[IO.File]::WriteAllText($path,($value|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))}
try{
    Case 'generated empty graph explicitly restores without a session or exports' {
        $f=Fixture 'valid';$check=Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)
        Require ($check.valid-and$check.project_sha256-ceq$f.project_sha256-and$check.preferences_sha256-ceq$f.preferences_sha256) 'Fixture hashes/readback differ'
        Require ($check.scope-ceq'stopped_ui_only'-and-not$check.qualified) 'Preflight must not claim runtime qualification'
    }
    Case 'false restore flag from the actual failed runner refuses' {
        $f=Fixture 'no-restore';$j=Get-Content $f.preferences_path -Raw|ConvertFrom-Json;$j.restore_last_project=$false;Set-Json $f.preferences_path $j
        Reject {Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)} 'Automatic default startup was admitted'
    }
    Case 'missing restored path refuses' {
        $f=Fixture 'missing-path';$j=Get-Content $f.preferences_path -Raw|ConvertFrom-Json;$j.last_project_path=$null;Set-Json $f.preferences_path $j
        Reject {Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)} 'Missing restore path admitted'
    }
    Case 'foreign or relative restored paths refuse' {
        foreach($path in @('stopped.moiren',(Join-Path $testRoot 'foreign.moiren'))){
            $f=Fixture ([guid]::NewGuid().ToString('N'));$j=Get-Content $f.preferences_path -Raw|ConvertFrom-Json;$j.last_project_path=$path;Set-Json $f.preferences_path $j
            Reject {Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)} 'Non-owned project path admitted'
        }
    }
    Case 'nonnull session and graphless projects refuse' {
        foreach($kind in @('session','graph')){
            $f=Fixture $kind;$j=Get-Content $f.project_path -Raw|ConvertFrom-Json
            if($kind-ceq'session'){$j.session=@{gain=0}}else{$j.graph=$null};Set-Json $f.project_path $j
            Reject {Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)} 'Unsafe project shape admitted'
        }
    }
    Case 'endpoint exports and nonempty topology refuse' {
        foreach($kind in @('exports','nodes','edges')){
            $f=Fixture $kind;$j=Get-Content $f.project_path -Raw|ConvertFrom-Json
            if($kind-ceq'exports'){$j.exports=@(@{name='unexpected'})}else{$j.graph.$kind=@(@{id=0})};Set-Json $f.project_path $j
            Reject {Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)} 'Nonempty UI-only resources admitted'
        }
    }
    Case 'wrong document versions and unknown fields refuse' {
        foreach($kind in @('version','unknown')){
            $f=Fixture $kind;$j=Get-Content $f.project_path -Raw|ConvertFrom-Json
            if($kind-ceq'version'){$j.version=1}else{$j|Add-Member NoteProperty surprise 1};Set-Json $f.project_path $j
            Reject {Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)} 'Changed fixed schema admitted'
        }
    }
    Case 'missing files malformed JSON and bounded input refuse' {
        $run=Join-Path $testRoot 'absent';New-Item -ItemType Directory -Path $run|Out-Null
        Reject {Test-MoirenStoppedUiStartup -RunDirectory $run} 'Missing documents admitted'
        foreach($text in @('{',(' ' * 65537))){$f=Fixture ([guid]::NewGuid().ToString('N'));[IO.File]::WriteAllText($f.project_path,$text);Reject {Test-MoirenStoppedUiStartup -RunDirectory (Split-Path $f.project_path -Parent)} 'Malformed/oversized project admitted'}
    }
    Case 'existing documents never overwrite and caller environment stays unchanged' {
        $before=$env:APPDATA;$f=Fixture 'immutable';$run=Split-Path $f.project_path -Parent
        Reject {New-MoirenStoppedUiStartup -RunDirectory $run} 'Existing fixture overwritten'
        Require ((Get-FileHash $f.project_path).Hash-ceq$f.project_sha256-and(Get-FileHash $f.preferences_path).Hash-ceq$f.preferences_sha256) 'Refusal changed existing data'
        Require ($env:APPDATA-ceq$before) 'Helper changed caller APPDATA'
    }
    Case 'junction ancestors refuse before creation or validation' {
        $target=Join-Path $testRoot 'owned-external';$outsideRun=Join-Path $target 'run'
        New-Item -ItemType Directory -Path $outsideRun|Out-Null
        $alias=Join-Path $testRoot 'alias';New-Item -ItemType Junction -Path $alias -Target $target|Out-Null;$script:junctions+=,$alias
        Reject {New-MoirenStoppedUiStartup -RunDirectory (Join-Path $alias 'run')} 'Junction ancestor admitted for writing'
        Require (@(Get-ChildItem -LiteralPath $outsideRun -Force).Count-eq0) 'Refusal wrote into external junction target'
        $f=Fixture 'linked-config';$run=Split-Path $f.project_path -Parent;$config=Split-Path $f.preferences_path -Parent
        $externalConfig=Join-Path $testRoot 'owned-external-config';Move-Item -LiteralPath $config -Destination $externalConfig
        New-Item -ItemType Junction -Path $config -Target $externalConfig|Out-Null;$script:junctions+=,$config
        Reject {Test-MoirenStoppedUiStartup -RunDirectory $run} 'Junction config ancestor admitted for validation'
    }
    Write-Output "$passed stopped-startup checks passed; filesystem-only, no service/window/audio/process execution"
}finally{
    $resolved=[IO.Path]::GetFullPath($testRoot);$temp=[IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if(-not$resolved.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase)-or-not(Split-Path $resolved -Leaf).StartsWith('moiren-stopped-startup-')){throw 'Refuse unexpected test cleanup path'}
    foreach($junction in $junctions){if(Test-Path -LiteralPath $junction){[IO.Directory]::Delete($junction)}}
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
