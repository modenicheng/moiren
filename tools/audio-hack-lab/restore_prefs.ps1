# Restores the 6 user per-app device preferences wiped by the slot-7 probe.
# Values transcribed verbatim from the registry dump taken before the wipe.
$root = 'HKCU:\SOFTWARE\Microsoft\Multimedia\Audio\DefaultEndpoint'
$suffix = '#{e6327cad-dcec-4949-ae8a-991e976a79d2}'
$dev = { param($g) "\\?\SWD#MMDEVAPI#{0.0.0.00000000}.$g$suffix" }

$entries = @(
    @{ k = '1282f995_0'; exe = '\Device\HarddiskVolume7\Program Files\Google\Chrome\Application\chrome.exe';
       ep = '{312105b7-6579-4597-95d6-b0ac319c56b8}'; p = '{FA207889-C84D-49CB-A609-E6B316CD50FA}' },
    @{ k = '68759d9d_0'; exe = '\Device\HarddiskVolume2\Games\Endfield Game\Endfield.exe';
       ep = '{b44b46d3-a33e-40c3-a989-5e1051edf1c5}'; p = '{0F385040-E700-4A23-8565-B28192CD596B}' },
    @{ k = 'c582767a_0'; exe = '\Device\HarddiskVolume7\Program Files (x86)\Tencent\QQMusic\QQMusic.exe';
       ep = '{b44b46d3-a33e-40c3-a989-5e1051edf1c5}'; p = '{0F385040-E700-4A23-8565-B28192CD596B}' },
    @{ k = 'd1485f2b_0'; exe = '\Device\HarddiskVolume7\Users\cheng\AppData\Roaming\Tencent\xwechat\XPlugin\Plugins\RadiumWMPF\18163\extracted\runtime\WeChatAppEx.exe';
       ep = '{099ca783-3ce2-4414-bdeb-7b9435da5196}'; p = '{2B47F081-F605-47AF-B0E2-126284206B1F}' },
    @{ k = 'ea80f80a_0'; exe = '\Device\HarddiskVolume7\Program Files\Fender\Studio Pro 8\Studio Pro.exe';
       ep = '{312105b7-6579-4597-95d6-b0ac319c56b8}'; p = '{FA207889-C84D-49CB-A609-E6B316CD50FA}' },
    @{ k = 'eb43d8b2_0'; exe = '\Device\HarddiskVolume7\Program Files\Image-Line\FL Studio 2024\FL64.exe';
       ep = '{099ca783-3ce2-4414-bdeb-7b9435da5196}'; p = '{2B47F081-F605-47AF-B0E2-126284206B1F}' }
)

foreach ($e in $entries) {
    $path = Join-Path $root $e.k
    New-Item -Path $path -Force | Out-Null
    $key = (Get-Item $path).OpenSubKey('', $true)
    $key.SetValue('', $e.exe)
    $key.Close()
    $d = "\\?\SWD#MMDEVAPI#{0.0.0.00000000}.$($e.ep)$suffix"
    foreach ($n in '000_000', '001_000') { New-ItemProperty -Path $path -Name $n -Value $d -PropertyType String -Force | Out-Null }
    foreach ($n in '000_000_p', '001_000_p') { New-ItemProperty -Path $path -Name $n -Value $e.p -PropertyType String -Force | Out-Null }
    Write-Host "restored $($e.k) <- $($e.exe)"
}
Write-Host '--- verify ---'
Get-ChildItem $root | ForEach-Object {
    $p = Get-ItemProperty $_.PSPath
    Write-Host ($_.PSChildName + '  <-  ' + $p.'(default)' + '  ->  ' + ($p.'000_000' -replace '.*\.\{(........-.*?\)\}.*', '$1'))
}
