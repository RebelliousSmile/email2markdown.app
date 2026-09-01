param(
    [Parameter(Mandatory = $true)]
    [string]$OutputDir,
    [string]$PfxPath = $env:EMAIL_TO_MARKDOWN_SIGNING_PFX,
    [string]$PfxPassword = $env:EMAIL_TO_MARKDOWN_SIGNING_PASSWORD
)

$ErrorActionPreference = "Stop"
$projectDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$outputPath = [System.IO.Path]::GetFullPath((Join-Path $projectDir $OutputDir))
$manifestPath = Join-Path $projectDir "packaging\windows\sparse-package\AppxManifest.xml"
$sdkRoot = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
$makeAppx = Get-ChildItem -LiteralPath $sdkRoot -Filter makeappx.exe -Recurse |
    Where-Object { $_.FullName -match "\\x64\\makeappx\.exe$" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $makeAppx) {
    throw "MakeAppx.exe est introuvable dans le SDK Windows"
}
$mt = Join-Path $makeAppx.DirectoryName "mt.exe"
if (-not (Test-Path -LiteralPath $mt)) {
    throw "mt.exe est introuvable à côté de MakeAppx.exe"
}

New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
$stagingPath = Join-Path ([System.IO.Path]::GetTempPath()) "email-to-markdown-identity-$PID"
New-Item -ItemType Directory -Path $stagingPath -Force | Out-Null
$packagePath = Join-Path $outputPath "email-to-markdown-identity.msix"

try {
    $executablePath = Join-Path $outputPath "email-to-markdown.exe"
    if (-not (Test-Path -LiteralPath $executablePath)) {
        throw "Le binaire à manifester est introuvable: $executablePath"
    }
    $applicationManifest = Join-Path $projectDir "packaging\windows\email-to-markdown.exe.manifest"
    & $mt -nologo -manifest $applicationManifest "-outputresource:$executablePath;#1"
    if ($LASTEXITCODE -ne 0) {
        throw "mt.exe a échoué avec le code $LASTEXITCODE"
    }

    Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $stagingPath "AppxManifest.xml")
    & $makeAppx.FullName pack /o /d $stagingPath /nv /p $packagePath
    if ($LASTEXITCODE -ne 0) {
        throw "MakeAppx a échoué avec le code $LASTEXITCODE"
    }

    if ($PfxPath) {
        $signTool = Join-Path $makeAppx.DirectoryName "signtool.exe"
        if (-not (Test-Path -LiteralPath $signTool)) {
            throw "SignTool.exe est introuvable à côté de MakeAppx.exe"
        }
        $signArgs = @("sign", "/fd", "SHA256", "/a", "/f", $PfxPath)
        if ($PfxPassword) {
            $signArgs += @("/p", $PfxPassword)
        }
        $signArgs += $packagePath
        & $signTool @signArgs
        if ($LASTEXITCODE -ne 0) {
            throw "SignTool a échoué avec le code $LASTEXITCODE"
        }
        Write-Host "Paquet d'identité Windows signé: $packagePath" -ForegroundColor Green
    } else {
        Write-Warning "Paquet d'identité non signé (développement uniquement): $packagePath"
    }

    $assetsPath = Join-Path $outputPath "Assets"
    New-Item -ItemType Directory -Path $assetsPath -Force | Out-Null
    $transparentPng = [Convert]::FromBase64String(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScL5WQAAAABJRU5ErkJggg=="
    )
    foreach ($name in @("StoreLogo.png", "Square44x44Logo.png", "Square150x150Logo.png")) {
        [System.IO.File]::WriteAllBytes((Join-Path $assetsPath $name), $transparentPng)
    }
} finally {
    $resolvedTemp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
    $resolvedStaging = [System.IO.Path]::GetFullPath($stagingPath)
    if ($resolvedStaging.StartsWith($resolvedTemp, [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolvedStaging -Recurse -Force -ErrorAction SilentlyContinue
    }
}
