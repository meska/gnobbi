# Assembla il pacchetto tester di Gnobbi per Windows (x64).
# Si esegue sulla VM di build con il toolchain Rust MSVC gia' configurato
# (vedi the WINDOWS_SPIKE notes): sorgente in E:\gnobbi, rustup/cargo su E:.
# Uso:  powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\make-tester-package.ps1
# Produce: dist\gnobbi-tester-windows-x64-<version>.zip + SHA256 su stdout e in dist\.

param(
    [string]$Version = "0.1.0",
    # I file DLL del runtime VC++ da includere (vuoto = nessuna): passare i
    # percorsi di vcruntime140.dll ecc. se dumpbin mostra import dinamici.
    [string[]]$RuntimeDlls = @()
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Write-Progress rompe la console su sessioni ssh
$env:RUSTUP_HOME = 'E:\rustup'
$env:CARGO_HOME = 'E:\cargo'
$env:PATH = "E:\cargo\bin;$env:PATH"
# gpui release compiles its shaders with fxc.exe; the SDK version on this VM:
$env:GPUI_FXC_PATH = 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\fxc.exe'

Push-Location (Join-Path $PSScriptRoot '..\..\')
$commit = (git rev-parse --short HEAD).Trim()
"== build (commit $commit) =="
& cargo build --locked --release -j4 -p matrix-desktop --features windows-vault
if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }

$stage = Join-Path (Get-Location) "dist\gnobbi-tester-windows-x64-$Version"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path $stage | Out-Null

Copy-Item target\release\gnobbi.exe $stage
Copy-Item packaging\windows\ISTRUZIONI_PER_TESTER.txt $stage
# vcruntime140.dll non è garantita su Windows pulito: la si affianca all'exe
# (distribuzione app-local, caricata dalla cartella del programma).
Copy-Item C:\Windows\System32\vcruntime140.dll $stage
foreach ($dll in $RuntimeDlls) { Copy-Item $dll $stage }

$zip = Join-Path (Get-Location) "dist\gnobbi-tester-windows-x64-$Version.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$stage\*" -DestinationPath $zip
Remove-Item $stage -Recurse -Force

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash
"$hash  gnobbi-tester-windows-x64-$Version.zip  (commit $commit)" | Tee-Object -FilePath "$zip.sha256"
"ZIP: $zip"
Pop-Location
