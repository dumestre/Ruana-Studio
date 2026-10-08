param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$BuildFolder,

    [string]$ReleaseNotes = "Versao de teste do Ruana Studio."
)

$ErrorActionPreference = "Stop"

if ($Version -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$' -or $Version -in @(".", "..")) {
    throw "Version must contain only letters, numbers, dots, dashes, and underscores."
}

$hubRoot = Split-Path -Parent $PSScriptRoot
$repoRoot = Split-Path -Parent $hubRoot
$targetRoot = Join-Path $repoRoot "target"
$buildRoot = if ([string]::IsNullOrWhiteSpace($BuildFolder)) {
    Join-Path $targetRoot "release"
} else {
    (Resolve-Path -LiteralPath $BuildFolder).Path
}
$exePath = Join-Path $buildRoot "ruana-studio.exe"
$cursorPath = Join-Path $buildRoot "cursors"
$outputDirectory = Join-Path $targetRoot "ruana-test-channel\$Version"
$stagingDirectory = Join-Path $targetRoot "ruana-test-staging-$Version"
$archiveName = "RuanaStudio-$Version-windows-x64.zip"
$archivePath = Join-Path $outputDirectory $archiveName
$manifestPath = Join-Path $outputDirectory "channel.json"

if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
    throw "Compiled Ruana Studio executable was not found: $exePath"
}
if (-not (Test-Path -LiteralPath $cursorPath -PathType Container)) {
    throw "Compiled cursor assets were not found: $cursorPath"
}
if (Test-Path -LiteralPath $stagingDirectory) {
    throw "Remove the previous staging folder before packaging: $stagingDirectory"
}

New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
New-Item -ItemType Directory -Path $stagingDirectory | Out-Null

try {
    Copy-Item -LiteralPath $exePath -Destination $stagingDirectory
    Copy-Item -LiteralPath $cursorPath -Destination $stagingDirectory -Recurse

    Compress-Archive -Path (Join-Path $stagingDirectory "*") -DestinationPath $archivePath -Force
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        throw "Failed to create build archive: $archivePath"
    }

    $sha256 = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $manifest = [ordered]@{
        version = $Version
        download_url = "https://builds.daime.space/ruana/stable/$archiveName"
        sha256 = $sha256
        release_notes = $ReleaseNotes
    }
    $manifestJson = $manifest | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText(
        $manifestPath,
        $manifestJson,
        [System.Text.UTF8Encoding]::new($false)
    )
} finally {
    if (Test-Path -LiteralPath $stagingDirectory) {
        Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
    }
}

Write-Host ""
Write-Host "Build channel package is ready:"
Write-Host "  $archivePath"
Write-Host "  $manifestPath"
Write-Host ""
Write-Host "Upload both files to the public R2 bucket path /ruana/stable/."
