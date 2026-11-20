# SPDX-License-Identifier: MIT
# Filesystem-only fixture for a stopped desktop control, not a general project
# validator. A missing restored project enables production automatic startup.
function Read-MoirenStoppedJson([string]$Path) {
    $file=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try {
        if($file.Length-gt65536){throw 'Stopped UI document exceeds 64 KiB'}
        $reader=[IO.StreamReader]::new($file,[Text.UTF8Encoding]::new($false,$true),$true)
        try {$text=$reader.ReadToEnd()} finally {$reader.Dispose()}
    } finally {$file.Dispose()}
    $text|ConvertFrom-Json -ErrorAction Stop
}

function Assert-MoirenStoppedKeys($Value,[string[]]$Expected,[string]$Label) {
    if($null-eq$Value-or$Value-isnot[Management.Automation.PSCustomObject]){throw ($Label+' must be an object')}
    $names=@($Value.PSObject.Properties.Name)
    if(@(Compare-Object ($Expected|Sort-Object) ($names|Sort-Object)).Count){throw ($Label+' differs from the fixed stopped UI schema')}
}

function Assert-MoirenStoppedAncestors([string]$Path) {
    $current=[IO.Path]::GetFullPath($Path)
    while($current){
        if(Test-Path -LiteralPath $current){
            if((Get-Item -LiteralPath $current).Attributes-band[IO.FileAttributes]::ReparsePoint){throw 'Stopped UI paths must not traverse reparse points'}
        }
        $parent=[IO.Path]::GetDirectoryName($current)
        if($parent-ceq$current){break};$current=$parent
    }
}

function Get-MoirenStoppedPaths([string]$RunDirectory) {
    if(-not[IO.Path]::IsPathRooted($RunDirectory)){throw 'Require absolute owned run directory'}
    $run=[IO.Path]::GetFullPath($RunDirectory).TrimEnd('\')
    Assert-MoirenStoppedAncestors $run
    if(-not(Test-Path -LiteralPath $run -PathType Container)){throw 'Run directory does not exist'}
    $directory=Get-Item -LiteralPath $run
    if($directory.Attributes-band[IO.FileAttributes]::ReparsePoint){throw 'Run directory must not be a reparse point'}
    @{run=$run;project=(Join-Path $run 'stopped.moiren');preferences=(Join-Path $run 'appdata\Moiren\preferences.json')}
}

function Test-MoirenStoppedUiStartup {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$RunDirectory)
    $paths=Get-MoirenStoppedPaths $RunDirectory
    foreach($path in @($paths.project,$paths.preferences)){
        Assert-MoirenStoppedAncestors $path
        $file=Get-Item -LiteralPath $path -ErrorAction Stop
        if($file.PSIsContainer-or($file.Attributes-band[IO.FileAttributes]::ReparsePoint)){throw 'Require ordinary stopped UI documents'}
    }
    $preferences=Read-MoirenStoppedJson $paths.preferences
    Assert-MoirenStoppedKeys $preferences @('version','language','restore_last_project','last_project_path') 'Preferences'
    if(($preferences.version-isnot[int]-and$preferences.version-isnot[long])-or$preferences.version-ne2-or$preferences.language-cne'en'-or$preferences.restore_last_project-isnot[bool]-or-not$preferences.restore_last_project){throw 'Require explicit English v2 project restoration'}
    if($preferences.last_project_path-isnot[string]-or-not[IO.Path]::IsPathRooted($preferences.last_project_path)-or[IO.Path]::GetFullPath($preferences.last_project_path)-cne$paths.project){throw 'Require exact owned absolute stopped project path'}
    $project=Read-MoirenStoppedJson $paths.project
    Assert-MoirenStoppedKeys $project @('version','name','session','graph','exports') 'Project'
    if(($project.version-isnot[int]-and$project.version-isnot[long])-or$project.version-ne2-or$project.name-cne'Stopped UI control'-or$null-ne$project.session){throw 'Require v2 stopped project with null session'}
    Assert-MoirenStoppedKeys $project.graph @('nodes','edges') 'Graph'
    if($project.exports-isnot[Array]-or$project.exports.Count-ne0-or$project.graph.nodes-isnot[Array]-or$project.graph.nodes.Count-ne0-or$project.graph.edges-isnot[Array]-or$project.graph.edges.Count-ne0){throw 'Stopped UI fixture must contain an empty graph and no endpoint exports'}
    [pscustomobject]@{valid=$true;qualified=$false;runtime_state_verified=$false;scope='stopped_ui_only';project_path=$paths.project;preferences_path=$paths.preferences;project_sha256=(Get-FileHash -LiteralPath $paths.project -Algorithm SHA256).Hash;preferences_sha256=(Get-FileHash -LiteralPath $paths.preferences -Algorithm SHA256).Hash}
}

function New-MoirenStoppedUiStartup {
    [CmdletBinding()]
    param([Parameter(Mandatory=$true)][string]$RunDirectory)
    $paths=Get-MoirenStoppedPaths $RunDirectory
    if((Test-Path -LiteralPath $paths.project)-or(Test-Path -LiteralPath $paths.preferences)){throw 'Stopped UI documents already exist; never overwrite'}
    $config=Split-Path $paths.preferences -Parent
    if(Test-Path -LiteralPath (Join-Path $paths.run 'appdata')){throw 'Require fresh isolated appdata subtree'}
    New-Item -ItemType Directory -Path $config -ErrorAction Stop|Out-Null
    $project=[ordered]@{version=2;name='Stopped UI control';session=$null;graph=[ordered]@{nodes=@();edges=@()};exports=@()}
    $preferences=[ordered]@{version=2;language='en';restore_last_project=$true;last_project_path=$paths.project}
    foreach($entry in @(@{path=$paths.project;value=$project},@{path=$paths.preferences;value=$preferences})){
        $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($entry.value|ConvertTo-Json -Depth 5))
        $file=[IO.File]::Open($entry.path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
        try{$file.Write($bytes,0,$bytes.Length);$file.Flush($true)}finally{$file.Dispose()}
    }
    Test-MoirenStoppedUiStartup -RunDirectory $paths.run
}
