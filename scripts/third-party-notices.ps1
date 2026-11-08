param([Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference = 'Stop'
$metadata = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$ids = @{}
foreach ($node in $metadata.resolve.nodes) { $ids[$node.id] = $true }
$output = [System.Text.StringBuilder]::new()
[void]$output.AppendLine('Mihomo Traffic - third-party notices')
[void]$output.AppendLine('Dependencies generated from Cargo.lock, including build dependencies; bundled asset licenses follow. The main project MIT license does not replace third-party licenses.')
foreach ($package in ($metadata.packages | Sort-Object name,version)) {
    if ($package.name -eq 'mihomo-traffic' -or -not $ids.ContainsKey($package.id)) { continue }
    [void]$output.AppendLine("`n============================================================")
    [void]$output.AppendLine("$($package.name) $($package.version) | $($package.license)")
    [void]$output.AppendLine("$($package.repository)")
    $directory = Split-Path -Parent $package.manifest_path
    $files = @(Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([._-].*)?$' })
    if ($package.license_file) {
        $licensePath = Join-Path $directory $package.license_file
        if (Test-Path -LiteralPath $licensePath -PathType Leaf) { $files += Get-Item -LiteralPath $licensePath }
    }
    foreach ($file in ($files | Sort-Object FullName -Unique)) {
        [void]$output.AppendLine("--- $($file.Name) ---")
        [void]$output.AppendLine([System.IO.File]::ReadAllText($file.FullName))
    }
    if ($files.Count -eq 0) { [void]$output.AppendLine("License declaration in package Cargo.toml: $($package.license). Source: https://crates.io/crates/$($package.name)/$($package.version)") }
}
$assetLicenses = Join-Path (Split-Path -Parent $PSScriptRoot) 'assets/licenses'
foreach ($assetLicense in (Get-ChildItem -LiteralPath $assetLicenses -Filter '*.txt' -File | Sort-Object Name)) {
    [void]$output.AppendLine("`n============================================================")
    [void]$output.AppendLine("Bundled icon asset: $($assetLicense.Name)")
    [void]$output.AppendLine([System.IO.File]::ReadAllText($assetLicense.FullName))
}
[System.IO.File]::WriteAllText($Destination, $output.ToString(), [System.Text.UTF8Encoding]::new($false))
