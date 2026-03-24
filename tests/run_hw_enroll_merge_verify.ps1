[CmdletBinding()]
param(
    [string]$CargoExe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "..\scripts\windows_common.ps1")

if (-not $CargoExe) {
    $CargoExe = Resolve-Cargo
}
$null = Resolve-CMake

$repoRoot = Get-RepoRoot -ScriptPath $PSCommandPath
$targetRoot = Initialize-CargoTargetDir -RepoRoot $repoRoot
Initialize-WindowsCompatHeaders -RepoRoot $repoRoot
Patch-NbisRsWindowsMsvc
Write-Host "Using CARGO_TARGET_DIR: $targetRoot"
Set-Location $repoRoot

Write-Host "[1/2] Running 6-scan enrollment package + verify test..."
Invoke-Cargo -CargoPath $CargoExe run --release --example hw_enroll_merge_verify --features hardware-tests

Write-Host "[2/2] Done."
