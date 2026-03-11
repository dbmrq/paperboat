# Paperboat installer for Windows
#
# Usage:
#   irm https://raw.githubusercontent.com/dbmrq/paperboat/main/scripts/install.ps1 | iex
#
# Options (via environment variables):
#   $env:PAPERBOAT_INSTALL_DIR - Installation directory (default: %LOCALAPPDATA%\Programs\paperboat)
#   $env:PAPERBOAT_VERSION     - Specific version to install (default: latest)
#   $env:PAPERBOAT_BINARY_PATH - Path to local binary (for testing, skips download)
#
# This script will:
# 1. Download the Windows binary from GitHub releases
# 2. Install it to a directory and add to PATH

$Repo = "dbmrq/paperboat"
$BinaryName = "paperboat"

function Write-Info { param($Message) Write-Host "info: " -ForegroundColor Blue -NoNewline; Write-Host $Message }
function Write-Warn { param($Message) Write-Host "warn: " -ForegroundColor Yellow -NoNewline; Write-Host $Message }
function Write-Success { param($Message) Write-Host "✓ " -ForegroundColor Green -NoNewline; Write-Host $Message }

function Write-ErrorAndWait {
    param($Message)
    Write-Host "error: " -ForegroundColor Red -NoNewline
    Write-Host $Message
    Write-Host ""
    Write-Host "Installation failed. Press Enter to exit..." -ForegroundColor Yellow
    Read-Host
    exit 1
}

# Global error handler to catch unexpected errors
trap {
    Write-Host ""
    Write-Host "error: " -ForegroundColor Red -NoNewline
    Write-Host $_.Exception.Message
    Write-Host ""
    Write-Host "Stack trace:" -ForegroundColor Yellow
    Write-Host $_.ScriptStackTrace
    Write-Host ""
    Write-Host "Installation failed. Press Enter to exit..." -ForegroundColor Yellow
    Read-Host
    exit 1
}

$ErrorActionPreference = "Stop"

function Get-LatestVersion {
    $response = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "paperboat-installer" }
    return $response.tag_name -replace '^v', ''
}

function Get-InstallDir {
    if ($env:PAPERBOAT_INSTALL_DIR) {
        return $env:PAPERBOAT_INSTALL_DIR
    }
    return Join-Path $env:LOCALAPPDATA "Programs\paperboat"
}

function Add-ToPath {
    param($Dir)
    
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -notlike "*$Dir*") {
        $newPath = "$currentPath;$Dir"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        $env:Path = "$env:Path;$Dir"
        Write-Info "Added $Dir to user PATH"
        Write-Warn "Restart your terminal for PATH changes to take effect"
    }
}

function Main {
    Write-Info "Installing Paperboat..."

    # Detect architecture
    $arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { Write-ErrorAndWait "32-bit Windows is not supported" }
    Write-Info "Detected: Windows $arch"

    # Get version
    $version = if ($env:PAPERBOAT_VERSION) { $env:PAPERBOAT_VERSION } else { Get-LatestVersion }
    if (-not $version) {
        Write-ErrorAndWait "Could not determine latest version"
    }
    Write-Info "Version: $version"

    # Get install directory
    $installDir = Get-InstallDir
    Write-Info "Install directory: $installDir"

    # Create install directory
    if (-not (Test-Path $installDir)) {
        New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    }

    # Download URL
    $archiveName = "${BinaryName}_${version}_Windows_${arch}.zip"
    $downloadUrl = "https://github.com/$Repo/releases/download/v$version/$archiveName"

    # Create temp directory
    $tmpDir = Join-Path $env:TEMP "paperboat-install-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $tmpDir -Force | Out-Null

    try {
        if ($env:PAPERBOAT_BINARY_PATH) {
            # Use local binary (for testing)
            Write-Info "Using local binary: $($env:PAPERBOAT_BINARY_PATH)"
            $binaryPath = $env:PAPERBOAT_BINARY_PATH
        }
        else {
            Write-Info "Downloading $archiveName..."
            $archivePath = Join-Path $tmpDir $archiveName
            Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath -UseBasicParsing

            Write-Info "Extracting..."
            Expand-Archive -Path $archivePath -DestinationPath $tmpDir -Force
            $binaryPath = Join-Path $tmpDir "$BinaryName.exe"
        }

        Write-Info "Installing to $installDir..."
        $destPath = Join-Path $installDir "$BinaryName.exe"
        Copy-Item -Path $binaryPath -Destination $destPath -Force

        # Add to PATH
        Add-ToPath -Dir $installDir

        Write-Success "Paperboat $version installed successfully!"
        Write-Host ""
        Write-Host "Get started with:"
        Write-Host "  paperboat --help"
    }
    finally {
        # Cleanup
        if (Test-Path $tmpDir) {
            Remove-Item -Path $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Main

