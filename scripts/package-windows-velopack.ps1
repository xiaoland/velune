param(
    [Parameter(Mandatory = $true)][string]$PackDirectory,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath (Join-Path $PackDirectory 'Velune.exe') -PathType Leaf)) {
    throw 'Velopack pack directory must contain Velune.exe.'
}
$productVersion = & python scripts/version.py --format full
if ($LASTEXITCODE -ne 0) {
    throw 'Could not read the canonical product version.'
}
if ($Version -cne $productVersion) {
    throw "Requested Velopack version '$Version' does not match version.txt '$productVersion'."
}
if (-not (Get-Command vpk -ErrorAction SilentlyContinue)) {
    throw 'The pinned Velopack CLI vpk 1.2.161 is not installed.'
}

if (Test-Path -LiteralPath $OutputDirectory) {
    Remove-Item -LiteralPath $OutputDirectory -Recurse -Force
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

& vpk pack `
    --packId Velune `
    --packVersion $Version `
    --packDir $PackDirectory `
    --mainExe Velune.exe `
    --runtime win-x64 `
    --channel win-x64-beta `
    --outputDir $OutputDirectory `
    --delta Disable `
    --noPortable
if ($LASTEXITCODE -ne 0) {
    throw "Velopack packaging failed with exit code $LASTEXITCODE."
}
