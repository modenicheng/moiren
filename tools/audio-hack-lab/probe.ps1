$ErrorActionPreference = 'Continue'
Write-Host '=== OS ==='
Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' |
    Select-Object ProductName, DisplayVersion, CurrentBuildNumber, UBR | Format-List

Write-Host '=== Elevation ==='
$id = [Security.Principal.WindowsIdentity]::GetCurrent()
Write-Host ('User: ' + $id.Name)
Write-Host ('Admin: ' + (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))

Write-Host '=== Sound devices (hardware) ==='
Get-CimInstance Win32_SoundDevice | Select-Object Name, Status | Format-List

Write-Host '=== Virtual audio drivers ==='
Get-CimInstance Win32_SystemDriver | Where-Object { $_.Name -match 'vb|vad|virtual|scream|voicemeeter|vbaudio' } |
    Select-Object Name, State, PathName | Format-List

Write-Host '=== Audio endpoints (MMDevices registry) ==='
$root = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio'
foreach ($dir in 'Render', 'Capture') {
    Write-Host ("--- $dir ---")
    Get-ChildItem -Path (Join-Path $root $dir) -ErrorAction SilentlyContinue | ForEach-Object {
        $ep = $_
        $props = Join-Path $ep.PSPath 'Properties'
        $name = $null
        if (Test-Path $props) {
            $name = (Get-ItemProperty $props -ErrorAction SilentlyContinue).'{a45c254e-df1c-4efd-8020-67d146a850e0},2'
        }
        $state = (Get-ItemProperty $ep.PSPath -ErrorAction SilentlyContinue).DeviceState
        Write-Host ("  {0}  state={1}  name={2}" -f $ep.PSChildName, $state, $name)
    }
}

Write-Host '=== audiodg ==='
Get-Process audiodg -ErrorAction SilentlyContinue | Select-Object Id, ProcessName | Format-List
try {
    $m = (Get-Process audiodg).Modules
    Write-Host ('Modules readable: ' + $m.Count)
} catch {
    Write-Host ('Modules access DENIED: ' + $_.Exception.Message.Trim())
}

Write-Host '=== bcdedit (testsigning) ==='
bcdedit /enum '{current}' | Select-String -Pattern 'testsigning|nointegritychecks'
