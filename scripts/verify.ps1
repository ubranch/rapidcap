param([switch]$AutomatedOnly)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$target = Join-Path (Split-Path $repo -Parent | Split-Path -Parent) 'work\rapidcap-target'
$env:CARGO_TARGET_DIR = $target

function Run([string[]]$Arguments) {
    & rustup run 1.97.1 cargo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "cargo $($Arguments -join ' ') failed" }
}

& rustup run 1.97.1 cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'cargo fmt failed' }
Run @('check', '--workspace', '--locked')
Run @('test', '--workspace', '--locked')
Run @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
Run @('test', '-p', 'rapidcap-capture', 'real_wgc_captures_primary_pixels', '--', '--ignored')
Run @('test', '-p', 'rapidcap-capture', 'real_video_records_and_finalizes', '--', '--ignored')
Run @('test', '-p', 'rapidcap-capture', 'real_gif_records_and_finalizes', '--', '--ignored')

if (-not $AutomatedOnly) {
    throw 'Manual acceptance still required: clipboard formats, 100%/150% DPI, keyboard focus, tray, hotkeys, and clean-account portability.'
}
Write-Output 'Automated verification passed; manual acceptance not claimed.'
