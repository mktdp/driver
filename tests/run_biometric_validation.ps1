[CmdletBinding()]
param(
    [string]$CargoExe,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ExampleArgs
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

Write-Host "[1/2] Running biometric validation harness..."
Invoke-Cargo -CargoPath $CargoExe run --release --example biometric_validation --features hardware-tests '--' @ExampleArgs

Write-Host "[2/2] Done."
