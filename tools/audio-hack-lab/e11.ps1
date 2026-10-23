param([string]$EndpointGuid)
$k = 'HKCU:\SOFTWARE\Microsoft\Multimedia\Audio\DefaultEndpoint\54dc0456_0'
$dev = "\\?\SWD#MMDEVAPI#{0.0.0.00000000}.$EndpointGuid#{e6327cad-dcec-4949-ae8a-991e976a79d2}"
foreach ($n in '000_000', '001_000') { Set-ItemProperty -Path $k -Name $n -Value $dev }
Write-Host "000_000 now: $((Get-ItemProperty $k).'000_000')"
