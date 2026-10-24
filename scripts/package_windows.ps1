param(
    [ValidateSet('Auto','CUDA','CPU')][string]$Backend='Auto',
    [string]$TargetDir,
    [string]$OutputDir,
    [switch]$InstallDependencies
)
$ErrorActionPreference='Stop'
$Root=Split-Path $PSScriptRoot -Parent
Set-Location $Root
if($InstallDependencies){& npm.cmd ci;if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}}
if($TargetDir){$env:CARGO_TARGET_DIR=[System.IO.Path]::GetFullPath($TargetDir)}
& "$PSScriptRoot/setup_runtime.ps1" -Backend $Backend
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
& npm.cmd run desktop:build
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
$Target=if($env:CARGO_TARGET_DIR){$env:CARGO_TARGET_DIR}else{Join-Path $Root 'src-tauri/target'}
& node "$PSScriptRoot/collect_licenses.mjs"
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
$Runtime=Get-Content "$Root/.cache/runtime/runtime.json" -Raw | ConvertFrom-Json
$Version=(Get-Content "$Root/package.json" -Raw | ConvertFrom-Json).version
$InstallResources=Join-Path $Root '.cache/installer'
New-Item -ItemType Directory -Force -Path $InstallResources | Out-Null
Copy-Item (Join-Path $Target 'release/DirectML.dll') (Join-Path $InstallResources 'DirectML.dll') -Force
& npm.cmd run desktop:installer
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
if(-not $OutputDir){$OutputDir=Join-Path $Root 'dist'}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$Installer=Get-ChildItem (Join-Path $Target 'release/bundle/nsis') -Filter '*-setup.exe' -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if(-not $Installer){throw 'NSIS installer was not generated'}
$Name="BlinkOCR-$Version-windows-x64-$($Runtime.backend.ToLowerInvariant())-setup.exe"
$Destination=Join-Path $OutputDir $Name
Copy-Item $Installer.FullName $Destination -Force
$Hash=(Get-FileHash $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
"$Hash  $Name"|Set-Content -Encoding ASCII "$Destination.sha256"
Write-Host "Installer: $Destination"
