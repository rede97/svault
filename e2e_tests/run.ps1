# Run E2E tests on Windows with proper environment isolation
#
# Usage:
#   .\run.ps1                           # Run all tests (debug build)
#   .\run.ps1 -Release                  # Run with release build
#   .\run.ps1 -Verbose -TestName "test_import"  # Verbose, only matching tests
#   .\run.ps1 -Cleanup                  # Cleanup temp directories after tests

param(
    [switch]$Release,
    [switch]$Verbose,
    [switch]$Cleanup,
    [string]$TestName = "",
    [string]$RamdiskSize = "256m"
)

$ErrorActionPreference = "Stop"

# Get script directory
$SCRIPT_DIR = Split-Path -Parent $MyInvocation.MyCommand.Definition
Set-Location $SCRIPT_DIR

# Determine Python path (try venv first, then system)
$PYTHON = "$SCRIPT_DIR\.venv\Scripts\python.exe"
$PYTEST = "$SCRIPT_DIR\.venv\Scripts\pytest.exe"

if (-not (Test-Path $PYTHON)) {
    $PYTHON = "python"
    $PYTEST = "pytest"
}

# Determine binary path and build args
if ($Release) {
    $BINARY = "$SCRIPT_DIR\..\target\release\svault.exe"
    $BUILD_ARGS = @("--release", "-p", "svault-cli", "-q")
} else {
    $BINARY = "$SCRIPT_DIR\..\target\debug\svault.exe"
    $BUILD_ARGS = @("-p", "svault-cli", "-q")
}

Write-Host "Checking svault binary ($(& { if ($Release) { "release" } else { "debug" } }))..."
if (-not (Test-Path $BINARY)) {
    Write-Host "Building svault..."
    Set-Location "$SCRIPT_DIR\.."
    & cargo build @BUILD_ARGS
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Failed to build svault"
        exit 1
    }
}

# Check for exiftool (used by some tests)
Write-Host "Checking exiftool..."
$exiftool = Get-Command exiftool -ErrorAction SilentlyContinue
if (-not $exiftool) {
    Write-Host "Warning: exiftool is not installed. Some tests may fail." -ForegroundColor Yellow
    Write-Host "Install it from: https://exiftool.org/"
}

# Build pytest arguments
$PYTEST_ARGS = @()
if ($Release) {
    $PYTEST_ARGS += "--release"
}
if ($Cleanup) {
    $PYTEST_ARGS += "--cleanup"
}
if ($Verbose) {
    $PYTEST_ARGS += "-v"
}
if ($TestName) {
    $PYTEST_ARGS += "-k"
    $PYTEST_ARGS += $TestName
}

# Run tests
Write-Host "Running tests..."
& $PYTEST @PYTEST_ARGS

exit $LASTEXITCODE
