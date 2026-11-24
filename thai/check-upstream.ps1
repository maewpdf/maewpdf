# Thai edition: is there an official PdfCraft release newer than the one this edition is built on?
#
#   powershell -ExecutionPolicy Bypass -File thai\check-upstream.ps1
#
# Reads the upstream release the edition is built on from the workspace version in Cargo.toml
# (MaewPDF's own number, thai\VERSION, counts separately from 1.0.0) and asks the official
# repository (storytold/pdfcraft) for its release tags. Read-only: it fetches
# nothing into the repository and changes nothing.

# Not 'Stop': Windows PowerShell turns git's stderr into a terminating error. Exit codes are
# checked instead.
$ErrorActionPreference = 'Continue'
$repo = 'https://github.com/storytold/pdfcraft.git'
$root = Split-Path -Parent $PSScriptRoot

$edition = (Get-Content (Join-Path $PSScriptRoot 'VERSION') -TotalCount 1).Trim()
$base = $null
$inPkg = $false
foreach ($line in Get-Content (Join-Path $root 'Cargo.toml')) {
    if ($line -match '^\[workspace\.package\]') { $inPkg = $true; continue }
    if ($line -match '^\[') { $inPkg = $false }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $base = $Matches[1]; break }
}
if (-not $base) { Write-Host 'Could not read the workspace version from Cargo.toml.' -ForegroundColor Red; exit 2 }

$tags = git ls-remote --tags $repo 2>$null
if ($LASTEXITCODE -ne 0 -or -not $tags) {
    Write-Host "Could not reach $repo (no internet, or GitHub is down). Try again later." -ForegroundColor Red
    exit 2
}

# Release tags only (vX.Y.Z), newest last.
$versions = $tags | ForEach-Object { if ($_ -match 'refs/tags/v(\d+\.\d+\.\d+)$') { [version]$Matches[1] } } | Sort-Object -Unique
$latest = $versions | Select-Object -Last 1

Write-Host "MaewPDF:             $edition (built on PdfCraft $base)"
Write-Host "Latest official:     $latest"

$newer = $versions | Where-Object { $_ -gt [version]$base }
if ($newer) {
    Write-Host ""
    Write-Host "New official version(s) out: $(($newer | ForEach-Object { "v$_" }) -join ', ')" -ForegroundColor Green
    if (Get-Command gh -ErrorAction SilentlyContinue) {
        $info = gh release view "v$latest" -R storytold/pdfcraft --json name,publishedAt,url 2>$null | ConvertFrom-Json
        if ($info) { Write-Host "$($info.name), published $($info.publishedAt)"; Write-Host $info.url }
    }
    exit 1
}

Write-Host ""
Write-Host "No newer official release." -ForegroundColor Yellow
# Work merged upstream since the release, not yet released.
$head = (git ls-remote $repo refs/heads/main 2>$null) -split '\s+' | Select-Object -First 1
if ($head) {
    Push-Location $root
    try {
        git cat-file -e "$head^{commit}" 2>$null
        if ($LASTEXITCODE -eq 0) {
            $count = git rev-list --count "v$latest..$head" 2>$null
            if ($count) { Write-Host "Official main has $count commit(s) since v$latest (as last fetched), not released yet." }
        } else {
            Write-Host "Official main has moved on since v$latest (run 'git fetch upstream' to count the commits)."
        }
    } finally { Pop-Location }
}
exit 0
