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

$go = Get-Command go -ErrorAction SilentlyContinue
if (-not $go) {
    throw "go was not found on PATH."
}

Write-Host "[1/3] Building Rust shared library..."
Invoke-Cargo -CargoPath $CargoExe build --release --features hardware-tests

Write-Host "[2/3] Running Go hardware tests..."
Add-DllSearchPath -RepoRoot $repoRoot -Profile "release"
$releaseTargetDir = Get-CargoProfileDir -RepoRoot $repoRoot -Profile "release"
Push-Location (Join-Path $repoRoot "go\fingerprint")
try {
    $env:GOCACHE = Join-Path $env:TEMP "go-build"
    $env:FP_HARDWARE_TESTS = "1"
    $env:CGO_LDFLAGS = "-L$releaseTargetDir"

    $msysGcc = "C:\msys64\mingw64\bin\gcc.exe"
    $msysGxx = "C:\msys64\mingw64\bin\g++.exe"
    if ((Test-Path $msysGcc) -and -not ($env:Path -split ';' | Where-Object { $_ -eq (Split-Path $msysGcc -Parent) })) {
        $env:Path = "$(Split-Path $msysGcc -Parent);$env:Path"
    }

    $goCgoEnabled = (& $go.Source env CGO_ENABLED).Trim()
    $goCc = (& $go.Source env CC).Trim()
    $goCcCmd = $null
    if (-not [string]::IsNullOrWhiteSpace($goCc)) {
        $goCcCmd = Get-Command $goCc -ErrorAction SilentlyContinue
    }

    if (($goCgoEnabled -ne "1" -or -not $goCcCmd) -and (Test-Path $msysGcc)) {
        Write-Host "  enabling cgo with MSYS2 MinGW for Go tests..."
        $env:CGO_ENABLED = "1"
        $env:CC = $msysGcc
        if (Test-Path $msysGxx) {
            $env:CXX = $msysGxx
        }
        $goCgoEnabled = (& $go.Source env CGO_ENABLED).Trim()
        $goCc = (& $go.Source env CC).Trim()
        if (-not [string]::IsNullOrWhiteSpace($goCc)) {
            $goCcCmd = Get-Command $goCc -ErrorAction SilentlyContinue
        }
    }

    if ($goCgoEnabled -ne "1" -or -not $goCcCmd) {
        throw "Go cgo toolchain unavailable (CGO_ENABLED=$goCgoEnabled, CC=$goCc). Install MSYS2 MinGW or configure CC for cgo."
    }

    & $go.Source test -v -tags hardwaretests -run TestHardware
    if ($LASTEXITCODE -ne 0) {
        throw "go test failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

Write-Host "[3/3] Done."
