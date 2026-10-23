# Optional developer convenience; dot-source to affect only this shell session.
param([switch]$UseOpenSslCache)
$taskBuildRoot = Split-Path -Parent $PSScriptRoot
$taskPerl = Join-Path $taskBuildRoot 'target\build-tools\strawberry\perl\bin\perl.exe'
if (Test-Path -LiteralPath $taskPerl) {
    $env:OPENSSL_SRC_PERL = $taskPerl
    Write-Output "Using portable build Perl: $taskPerl"
}
if ($UseOpenSslCache) {
    $taskPatterns = @(
        (Join-Path $taskBuildRoot 'native\target\debug\build\openssl-sys-*\out\openssl-build\install'),
        (Join-Path $taskBuildRoot 'target\debug\build\openssl-sys-*\out\openssl-build\install')
    )
    $taskCache = Get-ChildItem -Path $taskPatterns -Directory -ErrorAction SilentlyContinue |
        Where-Object { (Test-Path -LiteralPath (Join-Path $_.FullName 'lib\libcrypto.lib')) -and (Test-Path -LiteralPath (Join-Path $_.FullName 'lib\libssl.lib')) } |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $taskCache) { throw 'No local static OpenSSL cache exists. Build normally with native Windows Perl first.' }
    $env:OPENSSL_DIR = $taskCache.FullName
    $env:OPENSSL_STATIC = '1'
    $env:OPENSSL_NO_VENDOR = '1'
    Write-Output "Using existing local OpenSSL cache: $($taskCache.FullName)"
}
