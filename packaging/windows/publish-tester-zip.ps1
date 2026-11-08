# Pubblica lo zip tester Windows: cartella `latest` sulla VM (C:\ci\out\latest),
# archivio con retention e copia nel percorso raggiungibile da Marco
# (share Syncthing "gnobbi-build" sul runner Linux, come l'AppImage).
# Chiamato dal job windows di .gitea/workflows/ci.yml dopo make-tester-package.ps1.
# Uso: powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\publish-tester-zip.ps1
param(
    [string]$Zip = "",   # default: zip piu' recente in dist\
    [int]$ArchiveKeep = 5,
    [string]$Key = $env:CI_SHARE_KEY,
    [string]$Target = $env:CI_SHARE_TARGET
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Write-Progress rompe la console su sessioni ssh
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Push-Location $root
try {
    if (-not $Zip) {
        $Zip = (Get-ChildItem dist\*.zip | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
    }
    if (-not (Test-Path $Zip)) { throw "zip non trovato: $Zip (prima: make-tester-package.ps1)" }
    $sha256 = "$Zip.sha256"
    if (-not (Test-Path $sha256)) { throw "sha256 non trovato: $sha256" }

    $out = "C:\ci\out"
    $latest = Join-Path $out "latest"
    if (Test-Path $latest) { Remove-Item $latest -Recurse -Force }
    New-Item -ItemType Directory -Force $latest | Out-Null
    Copy-Item $Zip, $sha256 $latest
    Write-Output "LATEST: $latest"

    $archive = Join-Path $out "archive"
    New-Item -ItemType Directory -Force $archive | Out-Null
    Copy-Item $Zip, $sha256 $archive -Force
    Get-ChildItem $archive -Filter *.zip | Sort-Object LastWriteTime -Descending |
        Select-Object -Skip $ArchiveKeep | Remove-Item -Force
    # rimuovi gli .sha256 orfani degli zip potato
    Get-ChildItem $archive -Filter *.sha256 |
        Where-Object { -not (Test-Path ($_.FullName -replace '\.sha256$', '')) } | Remove-Item -Force
    Write-Output "ARCHIVE: $(@(Get-ChildItem $archive -Filter *.zip).Count) zip (max $ArchiveKeep)"

    if (-not (Test-Path $Key)) { throw "chiave ssh non trovata: $Key" }
    $sshOpts = "-o", "BatchMode=yes", "-o", "UserKnownHostsFile=$(Join-Path (Split-Path $Key) 'known_hosts')", "-o", "StrictHostKeyChecking=yes", "-i", $Key
    & scp @sshOpts $Zip $sha256 $Target
    if ($LASTEXITCODE -ne 0) { throw "scp verso $Target fallito (rc=$LASTEXITCODE)" }
    Write-Output "SHARE: $Target"
} finally {
    Pop-Location
}
