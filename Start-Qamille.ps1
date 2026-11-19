param([switch]$Demo)
$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
$executable = @(
    (Join-Path $projectRoot 'release\Qamille.exe'),
    (Join-Path $projectRoot 'app\target\release\qamille-desktop.exe'),
    (Join-Path $projectRoot 'app\target\debug\qamille-desktop.exe')
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $executable) {
    throw 'Run build-release.bat first to create the desktop app.'
}
$previousData = $env:QAMILLE_DATA
$previousDemo = $env:QAMILLE_DEMO
# Names from before the rebrandings, still read by the app: they must not pick the workspace.
$previousLegacyData = $env:SPARKLONE_DATA
$previousQemilleData = $env:QEMILLE_DATA
try {
    if ($Demo) {
        $env:QAMILLE_DATA = Join-Path (Split-Path -Parent $executable) 'preview\data'
        $env:QAMILLE_DEMO = '1'
    } else {
        $env:QAMILLE_DATA = $null
        $env:QAMILLE_DEMO = $null
    }
    $env:SPARKLONE_DATA = $null
    $env:QEMILLE_DATA = $null
    Start-Process -FilePath $executable -WorkingDirectory (Split-Path -Parent $executable) -WindowStyle Hidden
} finally {
    $env:QAMILLE_DATA = $previousData
    $env:QAMILLE_DEMO = $previousDemo
    $env:SPARKLONE_DATA = $previousLegacyData
    $env:QEMILLE_DATA = $previousQemilleData
}
