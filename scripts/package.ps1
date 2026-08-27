$ErrorActionPreference = 'Stop'

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$payload = Join-Path $repo 'assets\ffmpeg'
$required = @(
    'bin\ffmpeg.exe',
    'bin\ffprobe.exe',
    'LICENSE.txt',
    'BUILD.txt',
    'SOURCE.txt'
)
foreach ($relative in $required) {
    $file = Join-Path $payload $relative
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "Missing audited FFmpeg payload: $file"
    }
}

& rustup run 1.97.1 cargo build -p rapidcap-desktop --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }

$distRoot = Join-Path $repo 'dist'
$dist = Join-Path $distRoot 'RapidCap'
$resolvedParent = [IO.Path]::GetFullPath($distRoot)
if (-not $resolvedParent.StartsWith($repo + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Unsafe dist path: $resolvedParent"
}
if (Test-Path -LiteralPath $dist) {
    Remove-Item -LiteralPath $dist -Recurse -Force
}
New-Item -ItemType Directory -Path $dist | Out-Null

Copy-Item -LiteralPath (Join-Path $repo '..\..\work\rapidcap-target\release\RapidCap.exe') -Destination $dist
Copy-Item -LiteralPath (Join-Path $payload 'bin\ffmpeg.exe') -Destination $dist
Copy-Item -LiteralPath (Join-Path $payload 'bin\ffprobe.exe') -Destination $dist
Copy-Item -LiteralPath (Join-Path $payload 'LICENSE.txt') -Destination (Join-Path $dist 'FFMPEG-LICENSE.txt')
Copy-Item -LiteralPath (Join-Path $payload 'BUILD.txt') -Destination (Join-Path $dist 'FFMPEG-BUILD.txt')
Copy-Item -LiteralPath (Join-Path $payload 'SOURCE.txt') -Destination (Join-Path $dist 'FFMPEG-SOURCE.txt')
Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $dist

$probe = & (Join-Path $dist 'RapidCap.exe') --probe
if ($LASTEXITCODE -ne 0) { throw 'Packaged RapidCap --probe failed' }
$probe | ConvertFrom-Json | Out-Null
Get-ChildItem -LiteralPath $dist -File |
    Get-FileHash -Algorithm SHA256 |
    ForEach-Object { "{0}  {1}" -f $_.Hash.ToLowerInvariant(), (Split-Path $_.Path -Leaf) } |
    Set-Content -LiteralPath (Join-Path $dist 'SHA256SUMS.txt') -Encoding ascii

Write-Output $dist
