# build.ps1 - Script de compilation pour Email to Markdown
# Usage: .\scripts\build.ps1 [-Release] [-Features <features>]

param(
    [switch]$Release,
    [string]$Features = "",
    [switch]$Help
)

$ErrorActionPreference = "Stop"

if ($Help) {
    Write-Host @"
Usage: .\scripts\build.ps1 [options]

Options:
    -Release    Build en mode release (optimise)
    -Features   Features a activer (ex: "tray")
    -Help       Affiche cette aide

Exemples:
    .\scripts\build.ps1                      # Build debug
    .\scripts\build.ps1 -Release             # Build release
    .\scripts\build.ps1 -Features tray       # Build avec system tray
    .\scripts\build.ps1 -Release -Features tray
"@
    exit 0
}

# Fonction pour trouver Visual Studio
function Find-VisualStudio {
    $vswherePath = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"

    if (Test-Path $vswherePath) {
        $vsPath = & $vswherePath -latest -property installationPath
        if ($vsPath) {
            return $vsPath
        }
    }

    # Fallback: chercher manuellement
    $paths = @(
        "$env:ProgramFiles\Microsoft Visual Studio\2022\Community",
        "$env:ProgramFiles\Microsoft Visual Studio\2022\Professional",
        "$env:ProgramFiles\Microsoft Visual Studio\2022\Enterprise",
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\BuildTools",
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2019\Community",
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2019\BuildTools"
    )

    foreach ($path in $paths) {
        if (Test-Path $path) {
            return $path
        }
    }

    return $null
}

# Fonction pour trouver link.exe MSVC
function Find-MSVCLinker {
    param([string]$vsPath)

    $vcToolsPath = Get-ChildItem "$vsPath\VC\Tools\MSVC" -ErrorAction SilentlyContinue |
                   Sort-Object Name -Descending |
                   Select-Object -First 1

    if ($vcToolsPath) {
        $linkPath = "$($vcToolsPath.FullName)\bin\Hostx64\x64\link.exe"
        if (Test-Path $linkPath) {
            return $linkPath
        }
    }

    return $null
}

# Verifier si le link.exe actuel est le bon
function Test-LinkExe {
    if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
        return $false
    }
    $linkOutput = & link.exe 2>&1
    # Le link.exe MSVC affiche "Microsoft (R) Incremental Linker"
    return $linkOutput -match "Microsoft.*Linker"
}

Write-Host "=== Email to Markdown Build Script ===" -ForegroundColor Cyan

# Verifier le linker actuel
if (-not (Test-LinkExe)) {
    Write-Host "Le link.exe dans le PATH n'est pas le linker MSVC." -ForegroundColor Yellow
    Write-Host "Recherche de Visual Studio..." -ForegroundColor Yellow

    $vsPath = Find-VisualStudio

    if ($vsPath) {
        Write-Host "Visual Studio trouve: $vsPath" -ForegroundColor Green

        $linkerPath = Find-MSVCLinker $vsPath

        if ($linkerPath) {
            $linkerDir = Split-Path $linkerPath -Parent
            Write-Host "Ajout au PATH: $linkerDir" -ForegroundColor Green
            $env:PATH = "$linkerDir;$env:PATH"
        } else {
            Write-Host "Erreur: link.exe MSVC non trouve dans Visual Studio" -ForegroundColor Red
            Write-Host "Installez 'Desktop development with C++' dans Visual Studio Installer" -ForegroundColor Red
            exit 1
        }
    } else {
        Write-Host "Erreur: Visual Studio non trouve" -ForegroundColor Red
        Write-Host "Installez Visual Studio Build Tools depuis:" -ForegroundColor Yellow
        Write-Host "https://visualstudio.microsoft.com/downloads/" -ForegroundColor Yellow
        exit 1
    }
}

# Construire la commande cargo
$cargoArgs = @("build")

if ($Release) {
    $cargoArgs += "--release"
    Write-Host "Mode: Release" -ForegroundColor Green
} else {
    Write-Host "Mode: Debug" -ForegroundColor Green
}

if ($Features) {
    $cargoArgs += "--features"
    $cargoArgs += $Features
    Write-Host "Features: $Features" -ForegroundColor Green
}

Write-Host ""
Write-Host "Execution: cargo $($cargoArgs -join ' ')" -ForegroundColor Cyan
Write-Host ""

# Trouver cargo
$cargoPath = "$env:USERPROFILE\.cargo\bin\cargo.exe"
if (-not (Test-Path $cargoPath)) {
    $cargoPath = "cargo"  # Fallback au PATH
}

# Executer cargo
& $cargoPath @cargoArgs

if ($LASTEXITCODE -eq 0) {
    if ($Features -match "(^|,)\s*tray\s*(,|$)") {
        $extensionManifest = Join-Path $PSScriptRoot "..\packaging\windows\shell-extension\Cargo.toml"
        $extensionArgs = @("build", "--manifest-path", $extensionManifest)
        if ($Release) {
            $extensionArgs += "--release"
        }
        & $cargoPath @extensionArgs
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Echec de la compilation de l'extension Explorer Windows 11" -ForegroundColor Red
            exit $LASTEXITCODE
        }

        $cargoTomlPath = Join-Path $PSScriptRoot "..\Cargo.toml"
        $cargoTomlContent = Get-Content -LiteralPath $cargoTomlPath -Raw
        $versionMatch = [regex]::Match($cargoTomlContent, '(?m)^version\s*=\s*"([^"]+)"')
        if (-not $versionMatch.Success) {
            Write-Host "Impossible de lire la version depuis $cargoTomlPath" -ForegroundColor Red
            exit 1
        }
        $appVersion = $versionMatch.Groups[1].Value

        $extensionMode = if ($Release) { "release" } else { "debug" }
        $extensionSource = Join-Path $PSScriptRoot "..\packaging\windows\shell-extension\target\$extensionMode\email_to_markdown_shell_extension.dll"
        $extensionDestination = Join-Path $PSScriptRoot "..\target\$extensionMode\email-to-markdown-shell-extension-$appVersion.dll"
        Copy-Item -LiteralPath $extensionSource -Destination $extensionDestination -Force
        Write-Host "Extension Explorer: $extensionDestination" -ForegroundColor Cyan

        $issPath = Join-Path $PSScriptRoot "..\packaging\windows\email-to-markdown.iss"
        $issContent = Get-Content -LiteralPath $issPath -Raw
        $issContent = $issContent -replace '#define AppVersion "[^"]+"', "#define AppVersion `"$appVersion`""
        $issContent = $issContent -replace 'email-to-markdown-shell-extension-[^"]+\.dll', "email-to-markdown-shell-extension-$appVersion.dll"
        Set-Content -LiteralPath $issPath -Value $issContent -NoNewline
        Write-Host "email-to-markdown.iss mis a jour avec la version $appVersion" -ForegroundColor Cyan

        $identityOutput = if ($Release) { "target\release" } else { "target\debug" }
        & (Join-Path $PSScriptRoot "build-windows-identity.ps1") -OutputDir $identityOutput
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Echec de la construction du paquet d'identité Windows" -ForegroundColor Red
            exit $LASTEXITCODE
        }
    }

    Write-Host ""
    Write-Host "Build reussi!" -ForegroundColor Green

    $targetDir = if ($Release) { "target\release" } else { "target\debug" }
    Write-Host "Executable: $targetDir\email-to-markdown.exe" -ForegroundColor Cyan
} else {
    Write-Host ""
    Write-Host "Build echoue avec le code $LASTEXITCODE" -ForegroundColor Red
    exit $LASTEXITCODE
}
