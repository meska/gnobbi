# Assembla lo zip tester di Gnobbi per Windows (x64) dal binario release gia' costruito.
# La build la fa il chiamante (CI o sviluppatore):
#   cargo build --locked --release -p matrix-desktop --features windows-vault
# Uso: powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\make-tester-package.ps1
#      (opzioni -Exe <percorso> -Commit <sha> per usi fuori dalla CI)
# Produce: dist\gnobbi-windows-x64-<sha7>-<YYYYMMDD>.zip + .sha256 accanto, hash su stdout.
# Nome: sha7 + data UTC, come richiesto per i pacchetti tester. La pubblicazione sul sito
# (cartella download) copiera' questo zip come gnobbi-windows-x64-<build>.zip e link stabile.

param(
    [string]$Exe = "target\release\gnobbi.exe",
    [string]$Commit = ""
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Write-Progress rompe la console su sessioni ssh
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Push-Location $root
try {
    if (-not $Commit) { $Commit = (git rev-parse HEAD).Trim() }
    $sha7 = $Commit.Substring(0, 7)
    $date = [DateTime]::UtcNow.ToString('yyyyMMdd')
    $name = "gnobbi-windows-x64-$sha7-$date"

    if (-not (Test-Path $Exe)) {
        throw "exe non trovato: $Exe (prima: cargo build --locked --release -p matrix-desktop --features windows-vault)"
    }

    $stage = Join-Path $root "dist\$name"
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
    New-Item -ItemType Directory -Path $stage | Out-Null

    Copy-Item $Exe (Join-Path $stage 'gnobbi.exe')
    Copy-Item packaging\windows\ISTRUZIONI_PER_TESTER.txt $stage
    Copy-Item packaging\windows\gnobbi.ico $stage
    # vcruntime140.dll non è garantita su Windows pulito: app-local accanto all'exe.
    # _1 serve ai toolset recenti; c'è solo dove il redist è installato, quindi if present.
    foreach ($dll in 'vcruntime140.dll', 'vcruntime140_1.dll') {
        $src = "C:\Windows\System32\$dll"
        if (Test-Path $src) { Copy-Item $src $stage }
    }

    $zip = Join-Path $root "dist\$name.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path "$stage\*" -DestinationPath $zip
    Remove-Item $stage -Recurse -Force

    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    "$hash  $name.zip  (commit $Commit)" | Tee-Object -FilePath "$zip.sha256" | Write-Output
    Write-Output "ZIP: $zip"
} finally {
    Pop-Location
}
