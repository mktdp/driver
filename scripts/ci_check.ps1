[CmdletBinding()]
param(
    [string]$CargoExe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot "windows_common.ps1")

if (-not $CargoExe) {
    $CargoExe = Resolve-Cargo
}
$cmakeExe = Resolve-CMake
$targetRoot = Initialize-CargoTargetDir -RepoRoot $repoRoot
Initialize-WindowsCompatHeaders -RepoRoot $repoRoot
Patch-NbisRsWindowsMsvc -CargoPath $CargoExe
Write-Host "Using CARGO_TARGET_DIR: $targetRoot"
Write-Host "Using CMake: $cmakeExe"

Write-Host "[1/7] rustfmt check..."
Invoke-Cargo -CargoPath $CargoExe fmt --all --check

Write-Host "[2/7] clippy (deny warnings)..."
Invoke-Cargo -CargoPath $CargoExe clippy --release --all-targets --features "hardware-tests,debug-logging" '--' '-Dwarnings'

Write-Host "[3/7] rust tests (non-hardware)..."
Invoke-Cargo -CargoPath $CargoExe test --release

Write-Host "[4/7] compile hardware examples..."
Invoke-Cargo -CargoPath $CargoExe check --release --example hw_smoke_test --example biometric_validation --example hw_enroll_merge_verify --example hw_continuous_scan_dump --features hardware-tests

Write-Host "[5/7] build release shared library..."
Invoke-Cargo -CargoPath $CargoExe build --release --features hardware-tests

Write-Host "[6/7] compile C smoke test..."
$cSmoke = Build-CSmokeExecutable -RepoRoot $repoRoot -Profile "release" -OutputName "c_smoke_test_ci.exe"
Write-Host "  built $cSmoke"

Write-Host "[7/7] go tests (non-hardware)..."
$go = Get-Command go -ErrorAction SilentlyContinue
if (-not $go) {
    Write-Warning "go was not found on PATH. Skipping Go tests."
}
else {
    $goCache = Join-Path $env:TEMP "go-build"
    $releaseTargetDir = Get-CargoProfileDir -RepoRoot $repoRoot -Profile "release"
    Add-DllSearchPath -RepoRoot $repoRoot -Profile "release"

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
        Write-Warning "Skipping Go tests because cgo toolchain is unavailable (CGO_ENABLED=$goCgoEnabled, CC=$goCc)."
    }
    else {
        Push-Location (Join-Path $repoRoot "go\fingerprint")
        try {
            $env:GOCACHE = $goCache
            $env:CGO_LDFLAGS = "-L$releaseTargetDir"
            & $go.Source test ./...
            if ($LASTEXITCODE -ne 0) {
                throw "go test failed with exit code $LASTEXITCODE."
            }
        }
        finally {
            Pop-Location
        }
    }
}

Write-Host "CI checks passed."
