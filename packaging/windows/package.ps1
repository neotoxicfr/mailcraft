# Builds the MailCraft MSI from a release build.
#   Requires: Rust (MSVC), .NET SDK and WiX v5:  dotnet tool install --global wix --version 5.0.2
#   Usage:    pwsh packaging/windows/package.ps1 -Version 0.1.0
# Output:   dist/mailcraft-<version>-windows-x64.msi
param(
    [Parameter(Mandatory = $true)][string]$Version
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Push-Location $Root
try {
    cargo build --release --locked -p mailcraft
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $Exe = Join-Path $Root 'target\release\mailcraft.exe'
    # The icon is drawn by build.rs into its OUT_DIR; take the newest one.
    $Icon = Get-ChildItem (Join-Path $Root 'target\release\build') -Recurse -Filter mailcraft.ico |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
    if (-not $Icon) { throw 'mailcraft.ico not found (build.rs did not run?)' }
    $Dist = Join-Path $Root 'dist'
    New-Item -ItemType Directory -Force $Dist | Out-Null
    $Msi = Join-Path $Dist "mailcraft-$Version-windows-x64.msi"
    wix build (Join-Path $PSScriptRoot 'mailcraft.wxs') -arch x64 `
        -d "Version=$Version" -d "Exe=$Exe" -d "Icon=$Icon" -d "Root=$Root" -o $Msi
    if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }
    # wix writes its debug symbols next to the MSI; keep them out of the release assets.
    Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
    Write-Output $Msi
} finally {
    Pop-Location
}
