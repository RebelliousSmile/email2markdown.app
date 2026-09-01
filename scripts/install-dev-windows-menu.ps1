param(
    [string]$OutputDir = "target\release"
)

$ErrorActionPreference = "Stop"
$currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$currentPrincipal = New-Object Security.Principal.WindowsPrincipal($currentIdentity)
if (-not $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $powershell = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -OutputDir `"$OutputDir`""
    $elevated = Start-Process -FilePath $powershell -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    exit $elevated.ExitCode
}

$projectDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$outputPath = [System.IO.Path]::GetFullPath((Join-Path $projectDir $OutputDir))
$packagePath = Join-Path $outputPath "email-to-markdown-identity.msix"
$executablePath = Join-Path $outputPath "email-to-markdown.exe"
foreach ($required in @($packagePath, $executablePath)) {
    if (-not (Test-Path -LiteralPath $required)) {
        throw "Fichier de développement introuvable: $required"
    }
}

$certificate = Get-ChildItem Cert:\CurrentUser\My |
    Where-Object {
        $_.Subject -eq "CN=FXGuillois" -and
        $_.FriendlyName -eq "Email to Markdown development signing" -and
        $_.HasPrivateKey -and
        $_.NotAfter -gt (Get-Date).AddDays(1)
    } |
    Select-Object -First 1
if (-not $certificate) {
    $certificate = New-SelfSignedCertificate `
        -Type CodeSigningCert `
        -Subject "CN=FXGuillois" `
        -FriendlyName "Email to Markdown development signing" `
        -KeyExportPolicy NonExportable `
        -CertStoreLocation Cert:\CurrentUser\My
}

$trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople |
    Where-Object { $_.Thumbprint -eq $certificate.Thumbprint } |
    Select-Object -First 1
if (-not $trusted) {
    $publicCertificate = Join-Path ([System.IO.Path]::GetTempPath()) "email-to-markdown-$PID.cer"
    try {
        Export-Certificate -Cert $certificate -FilePath $publicCertificate | Out-Null
        Import-Certificate -FilePath $publicCertificate -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    } finally {
        Remove-Item -LiteralPath $publicCertificate -Force -ErrorAction SilentlyContinue
    }
}

$sdkRoot = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
$signTool = Get-ChildItem -LiteralPath $sdkRoot -Filter signtool.exe -Recurse |
    Where-Object { $_.FullName -match "\\x64\\signtool\.exe$" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $signTool) {
    throw "SignTool.exe est introuvable dans le SDK Windows"
}
& $signTool.FullName sign /fd SHA256 /sha1 $certificate.Thumbprint /s My $packagePath
if ($LASTEXITCODE -ne 0) {
    throw "SignTool a échoué avec le code $LASTEXITCODE"
}

& $executablePath shell install
if ($LASTEXITCODE -ne 0) {
    throw "L'enregistrement du menu Windows 11 a échoué avec le code $LASTEXITCODE"
}
