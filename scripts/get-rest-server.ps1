param([Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference = 'Stop'
# Optional, explicit helper installation. Does not modify PATH or run a service.
if (-not [IO.Path]::IsPathFullyQualified($Destination)) { throw 'Choose an absolute new destination directory' }
if (Test-Path -LiteralPath $Destination) { throw 'Destination already exists; nothing was overwritten' }
$work = Join-Path ([IO.Path]::GetTempPath()) ('containersafe-helper-' + [guid]::NewGuid())
[IO.Directory]::CreateDirectory($work) | Out-Null
try {
    $archive = Join-Path $work 'rest-server.zip'
    Invoke-WebRequest -Uri 'https://github.com/restic/rest-server/releases/download/v0.14.0/rest-server_0.14.0_windows_amd64.zip' -OutFile $archive
    $expected = '98e31e33816861722e0f04c7f56811a3ff2f37e006def217ad6eda8e438c9d82'
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'Official helper checksum mismatch' }
    Expand-Archive -LiteralPath $archive -DestinationPath $Destination
    Write-Output "Verified rest-server 0.14.0 extracted to $Destination. Select rest-server.exe in the backup plan."
} finally {
    # Only this invocation's exact temporary file/directory; no recursive deletion.
    $archive = Join-Path $work 'rest-server.zip'
    if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive }
    Remove-Item -LiteralPath $work
}
