param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-p]{32}$')]
    [string]$ExtensionId,

    [string]$NativeHostExe = "",

    [ValidateSet("Both", "Chrome", "Edge")]
    [string]$Browser = "Both",

    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Resolve-Path (Join-Path $ScriptDir "..")
$InstallDir = Join-Path $env:LOCALAPPDATA "StorDown\BrowserIntegration"
$InstalledHost = Join-Path $InstallDir "stordown-native-host.exe"
$ManifestPath = Join-Path $InstallDir "native-messaging-host.json"

$RegistryKeys = @()
if ($Browser -eq "Both" -or $Browser -eq "Chrome") {
    $RegistryKeys += "HKCU:\Software\Google\Chrome\NativeMessagingHosts\cloud.hoststorm.stordown"
}
if ($Browser -eq "Both" -or $Browser -eq "Edge") {
    $RegistryKeys += "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\cloud.hoststorm.stordown"
}

if ($Uninstall) {
    foreach ($Key in $RegistryKeys) {
        if (Test-Path $Key) {
            Remove-Item -Path $Key -Recurse -Force
        }
    }

    if (Test-Path $ManifestPath) {
        Remove-Item $ManifestPath -Force
    }

    Write-Host "Integração StorDown removida para: $Browser" -ForegroundColor Green
    exit 0
}

if ([string]::IsNullOrWhiteSpace($NativeHostExe)) {
    $Candidates = @(
        (Join-Path $RepoRoot "target\release\stordown-native-host.exe"),
        (Join-Path $RepoRoot "target\debug\stordown-native-host.exe")
    )

    $NativeHostExe = $Candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}

if ([string]::IsNullOrWhiteSpace($NativeHostExe) -or -not (Test-Path $NativeHostExe)) {
    Write-Host "Compilando StorDown Native Host..." -ForegroundColor Cyan
    Push-Location $RepoRoot
    try {
        cargo build -p stordown-native-host --release
    }
    finally {
        Pop-Location
    }

    $NativeHostExe = Join-Path $RepoRoot "target\release\stordown-native-host.exe"
}

$NativeHostExe = (Resolve-Path $NativeHostExe).Path
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item -Path $NativeHostExe -Destination $InstalledHost -Force

$InstalledHost = (Resolve-Path $InstalledHost).Path

$Manifest = [ordered]@{
    name = "cloud.hoststorm.stordown"
    description = "StorDown browser capture bridge"
    path = $InstalledHost
    type = "stdio"
    allowed_origins = @(
        "chrome-extension://$ExtensionId/"
    )
}

$Manifest | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 $ManifestPath

foreach ($Key in $RegistryKeys) {
    New-Item -Path $Key -Force | Out-Null
    Set-Item -Path $Key -Value $ManifestPath
}

Write-Host ""
Write-Host "StorDown Browser Integration instalada." -ForegroundColor Green
Write-Host "Navegador(es): $Browser"
Write-Host "Manifest: $ManifestPath"
Write-Host "Native Host: $InstalledHost"
Write-Host "Extensão: $ExtensionId"
Write-Host ""
Write-Host "Abra o StorDown Desktop e clique em Testar no popup da extensão."
