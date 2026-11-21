# The Authenticode signature of one file (docs/edizioni.md, "Firma e aggiornamenti"). Tauri's bundler
# calls it (bundle.windows.signCommand in tauri.signing.conf.json, which build-release.bat adds with
# QAMILLE_SIGNING_PFX) for the program it puts in the installer, the uninstaller and the installer, before
# the updater signs the installer; build-release.bat calls it for the portable Qamille.exe.
#
#   QAMILLE_SIGNING_PFX            the certificate with its private key, a .pfx file
#   QAMILLE_SIGNING_PFX_PASSWORD   its password (may be empty)
#   QAMILLE_SIGNING_TIMESTAMP_URL  the RFC 3161 timestamp server (default DigiCert's)
#
# The password comes from the environment only and never reaches a command line or a log: the PFX goes
# into a certificate store of this one signature, under the current user, signtool signs by thumbprint
# from there, and the store, the certificate and its private key are removed afterwards.
# Windows PowerShell 5.1 (powershell.exe), which every Windows has.
param([Parameter(Mandatory = $true)][string]$File)
$ErrorActionPreference = 'Stop'

$pfx = $env:QAMILLE_SIGNING_PFX
if (-not $pfx -or -not (Test-Path -LiteralPath $pfx -PathType Leaf)) { throw "QAMILLE_SIGNING_PFX does not name a .pfx file: '$pfx'" }
if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { throw "nothing to sign at '$File'" }
$timestamp = if ($env:QAMILLE_SIGNING_TIMESTAMP_URL) { $env:QAMILLE_SIGNING_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }

# The newest signtool of the Windows SDK.
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$signtool = Get-ChildItem -LiteralPath $kits -Directory -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -match '^\d+(\.\d+){3}$' } |
  Sort-Object { [version]$_.Name } -Descending |
  ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
  Where-Object { Test-Path -LiteralPath $_ } |
  Select-Object -First 1
if (-not $signtool) { throw "no signtool.exe in $kits (the Windows SDK)" }

$secret = New-Object System.Security.SecureString
foreach ($character in [char[]][string]$env:QAMILLE_SIGNING_PFX_PASSWORD) { $secret.AppendChar($character) }
$secret.MakeReadOnly()

# A store of its own, never one that holds other certificates: everything in it goes afterwards. The
# certificate drive does not create stores for the current user; opening one for writing does, and it
# lives under HKCU\Software\Microsoft\SystemCertificates.
$store = "QamilleSigning$PID"
$path = "Cert:\CurrentUser\$store"
$registry = "HKCU:\Software\Microsoft\SystemCertificates\$store"
$opened = New-Object System.Security.Cryptography.X509Certificates.X509Store($store, 'CurrentUser')
$opened.Open('ReadWrite')
$opened.Close()
try {
  $imported = @(Import-PfxCertificate -FilePath $pfx -CertStoreLocation $path -Password $secret)
  $signer = $imported | Where-Object { $_.HasPrivateKey } | Select-Object -First 1
  if (-not $signer) { throw "$pfx holds no certificate with its private key" }
  # A timestamp server can fail for a moment: three tries.
  foreach ($try in 1..3) {
    & $signtool sign /s $store /sha1 $signer.Thumbprint /fd SHA256 /tr $timestamp /td SHA256 /d Qamille $File
    if ($LASTEXITCODE -eq 0) { break }
    if ($try -eq 3) { throw "signtool could not sign $File" }
    Start-Sleep -Seconds 10
  }
}
finally {
  # The certificate with its private key, then the store itself.
  Get-ChildItem -LiteralPath $path -ErrorAction SilentlyContinue |
    ForEach-Object { Remove-Item -LiteralPath $_.PSPath -DeleteKey -ErrorAction SilentlyContinue }
  Remove-Item -LiteralPath $registry -Recurse -Force -ErrorAction SilentlyContinue
}
