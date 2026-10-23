param([string]$Wix = 'wix', [string]$BinaryDirectory = 'target\release')
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location $taskRoot
try {
    $taskMeta = python scripts/release_metadata.py | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Release version validation failed' }
    $taskStage = Join-Path $taskRoot 'target\package-windows'
    New-Item -ItemType Directory -Force -Path $taskStage,dist | Out-Null
    Copy-Item -LiteralPath (Join-Path $BinaryDirectory 'containersafe-native.exe') -Destination (Join-Path $taskStage 'ContainerSafeDesktop.exe')
    Copy-Item -LiteralPath (Join-Path $BinaryDirectory 'containersafe.exe') -Destination $taskStage
    Copy-Item -LiteralPath README.md -Destination $taskStage
    python scripts/third_party_notices.py --target x86_64-pc-windows-msvc --output "$taskStage\THIRD-PARTY-NOTICES.txt"
    if ($LASTEXITCODE -ne 0) { throw 'Third-party notices generation failed' }
    $taskMsi = Join-Path $taskRoot "dist\ContainerSafe-$($taskMeta.version)-windows-x64.msi"
    & $Wix build packaging/windows/ContainerSafe.wxs -arch x64 -d "ProductVersion=$($taskMeta.package_version)" -d "StageDir=$taskStage" -o $taskMsi
    if ($LASTEXITCODE -ne 0) { throw 'WiX MSI build failed' }
    Write-Output $taskMsi
} finally { Pop-Location }
