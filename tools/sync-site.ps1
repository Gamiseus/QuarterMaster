# Copies what the site publishes from this workspace into the site repository
# (gamiseus.github.io): the modding wiki, the changelog and the editor schemas. The site's build
# reads them from its content/ folder.
#
#   tools\sync-site.ps1            copy whatever changed, then commit the site repository
#   tools\sync-site.ps1 -Check     only say whether the copies are current (exit code 1 if not)
#   tools\sync-site.ps1 -Site DIR  use a site repository somewhere else
param(
    [string]$Site,
    [switch]$Check
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$workspace = Split-Path $root -Parent

if (-not $Site) {
    # The second name is the repository's first, misspelled name.
    foreach ($name in "gamiseus.github.io", "gamiseus.githhub.io") {
        $candidate = Join-Path $workspace $name
        if (Test-Path $candidate) { $Site = $candidate; break }
    }
}
if (-not $Site -or -not (Test-Path $Site)) {
    if ($Check) {
        Write-Host "The site repository is not next to this workspace; nothing to check."
        exit 0
    }
    Write-Host "The site repository was not found. Pass -Site <folder>."
    exit 1
}

$wiki = Join-Path $workspace "docs\QUARTERMASTER_MODDING_WIKI.md"
$files = @(
    @{ From = $wiki; To = "content\wiki.md" },
    @{ From = (Join-Path $root "CHANGELOG.md"); To = "content\changelog.md" }
)
foreach ($schema in Get-ChildItem (Join-Path $root "schemas") -Filter *.json | Sort-Object Name) {
    $files += @{ From = $schema.FullName; To = "content\schemas\$($schema.Name)" }
}

$stale = @()
foreach ($f in $files) {
    if (-not (Test-Path $f.From)) {
        Write-Host "Missing: $($f.From)"
        exit 1
    }
    $to = Join-Path $Site $f.To
    $same = (Test-Path $to) -and ((Get-FileHash $f.From).Hash -eq (Get-FileHash $to).Hash)
    if ($same) { continue }
    $stale += $f.To
    if (-not $Check) {
        New-Item -ItemType Directory -Force (Split-Path $to -Parent) | Out-Null
        Copy-Item $f.From $to -Force
    }
}

if ($stale.Count -eq 0) {
    Write-Host "The site's content is current."
    exit 0
}
if ($Check) {
    Write-Host "The site's content is out of date: $($stale -join ', ')"
    Write-Host "Run tools\sync-site.ps1, then commit the site repository."
    exit 1
}
Write-Host "Updated in ${Site}: $($stale -join ', ')"
Write-Host "Commit and push the site repository to publish it."
exit 0
