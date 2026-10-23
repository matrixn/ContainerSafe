param([string]$Version)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$source = Join-Path $repo 'src\lib.rs'
$text = [IO.File]::ReadAllText($source)
$current = [regex]::Match($text, 'pub const VERSION: &str = "([0-9]+\.[0-9]{2})";').Groups[1].Value
if (-not $current) { throw 'Application display version was not found' }
$culture = [Globalization.CultureInfo]::InvariantCulture
if (-not $Version) { $Version = ([decimal]::Parse($current, $culture) + [decimal]0.01).ToString('0.00', $culture) }
if ($Version -notmatch '^([0-9]+)\.([0-9]{2})$') { throw 'Use a display version such as 0.09 or 0.10' }
$semver = '{0}.{1}.0' -f [int]$Matches[1], [int]$Matches[2]
$utf8 = [Text.UTF8Encoding]::new($false)
foreach ($relative in @('Cargo.toml', 'native\Cargo.toml')) {
    $path = Join-Path $repo $relative
    $manifest = [IO.File]::ReadAllText($path)
    $manifest = [regex]::Replace($manifest, '(?m)^version = "[^"]+"', ('version = "' + $semver + '"'))
    [IO.File]::WriteAllText($path, $manifest, $utf8)
}
[IO.File]::WriteAllText($source, $text.Replace(('pub const VERSION: &str = "' + $current + '";'), ('pub const VERSION: &str = "' + $Version + '";')), $utf8)
Write-Output "ContainerSafe $current -> $Version (Cargo $semver). Run Cargo checks to refresh lockfiles."
