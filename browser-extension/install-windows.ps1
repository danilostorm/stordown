param(
    [Parameter(Mandatory = $true)]
    [string]$ExtensionId,

    [string]$NativeHostExe = ""
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path (Join-Path $ScriptDir "..")

if ([string]::IsNullOrWhiteSpace($NativeHostExe)) {
    $NativeHostExe = Join-Path $RepoRoot "target\release\stordown-native-host.exe"
}

if (-not (Test-Path $NativeHostExe)) {
    Write-Host "Compilando StorDown Native Host..." -ForegroundColor Cyan
    Push-Location $RepoRoot
    try {
        cargo build -p stordown-native-host --release
    }
    finally {
        Pop-Location
    }
}

$NativeHostExe = (Resolve-Path $NativeHostExe).Path
$InstallDir = Join-Path $env:LOCALAPPDATA "StorDown"
$ManifestPath = Join-Path $InstallDir "native-messaging-host.json"

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$Manifest = [ordered]@{
    name = "cloud.hoststorm.stordown"
    description = "StorDown browser capture bridge"
    path = $NativeHostExe
    type = "stdio"
    allowed_origins = @(
        "chrome-extension://$ExtensionId/"
    )
}

$Manifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 $ManifestPath

$RegistryKeys = @(
    "HKCU:\Software\Google\Chrome\NativeMessagingHosts\cloud.hoststorm.stordown",
    "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\cloud.hoststorm.stordown"
)

foreach ($Key in $RegistryKeys) {
    New-Item -Path $Key -Force | Out-Null
    Set-Item -Path $Key -Value $ManifestPath
}

Write-Host ""
Write-Host "StorDown Native Host registrado." -ForegroundColor Green
Write-Host "Manifest: $ManifestPath"
Write-Host "Executavel: $NativeHostExe"
Write-Host "Extensao: $ExtensionId"
Write-Host ""
Write-Host "Abra o StorDown Desktop e use o botao Testar no popup da extensao."
