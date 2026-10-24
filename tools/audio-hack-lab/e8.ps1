param([string]$Action = 'add')
$nt = '\Device\HarddiskVolume2\coding\moiren\tools\audio-hack-lab\lab\target\release\audio-hack-lab.exe'
$k = 'HKCU:\SOFTWARE\Microsoft\Multimedia\Audio\DefaultEndpoint\zztest01_0'
$dev = '\\?\SWD#MMDEVAPI#{0.0.0.00000000}.{3543a63a-4a4d-48ae-ba27-f2155d619370}#{e6327cad-dcec-4949-ae8a-991e976a79d2}'
$p = '{0F385040-E700-4A23-8565-B28192CD596B}'
if ($Action -eq 'add') {
    New-Item -Path $k -Force | Out-Null
    (Get-Item $k).OpenSubKey('', $true).SetValue('', $nt)
    foreach ($n in '000_000', '001_000') { New-ItemProperty -Path $k -Name $n -Value $dev -PropertyType String -Force | Out-Null }
    foreach ($n in '000_000_p', '001_000_p') { New-ItemProperty -Path $k -Name $n -Value $p -PropertyType String -Force | Out-Null }
    Write-Host 'key written'
    Get-Item $k | Select-Object -ExpandProperty Property
} else {
    Remove-Item -Path $k -Recurse -Force
    Write-Host 'key removed'
}
