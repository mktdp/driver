[CmdletBinding()]
param(
    [string]$CargoExe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $repoRoot "scripts\windows_common.ps1")

if (-not $CargoExe) {
    $CargoExe = Resolve-Cargo
}
$null = Resolve-CMake
$targetRoot = Initialize-CargoTargetDir -RepoRoot $repoRoot
Initialize-WindowsCompatHeaders -RepoRoot $repoRoot
Patch-NbisRsWindowsMsvc
Write-Host "Using CARGO_TARGET_DIR: $targetRoot"

Write-Host "[1/4] Building Rust library..."
Invoke-Cargo -CargoPath $CargoExe build --release --features hardware-tests

Write-Host "[2/4] Compiling C smoke test..."
$exePath = Build-CSmokeExecutable -RepoRoot $repoRoot -Profile "release" -OutputName "c_smoke_test.exe"

Write-Host "[3/4] Running C smoke test..."
Add-DllSearchPath -RepoRoot $repoRoot -Profile "release"
& $exePath
if ($LASTEXITCODE -ne 0) {
    throw "C smoke test failed with exit code $LASTEXITCODE."
}

Write-Host "[4/4] Done."
