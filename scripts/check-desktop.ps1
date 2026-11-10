param(
    [ValidateSet('Ui', 'Taskbar')][string]$Suite,
    [string]$Filter,
    [switch]$List,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
if (-not $Suite) { throw 'Choose a desktop test suite with -Suite Ui or -Suite Taskbar. Add -List to list tests without opening windows.' }
. (Join-Path $PSScriptRoot 'windows-common.ps1')
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
$oldInterfaceExecutable = $env:MIHOMO_DESKTOP_TEST_EXE
try {
    $buildArguments = @('test', '--locked', '--no-run', '--message-format=json-render-diagnostics')
    if ($Release) { $buildArguments += '--release' }
    $buildMessages = @(cargo @buildArguments)
    if ($LASTEXITCODE -ne 0) { throw 'Desktop test build failed' }
    # Use Cargo's artifact path, rather than selecting an old binary by timestamp.
    $testExecutable = $buildMessages | ForEach-Object {
        $message = $_ | ConvertFrom-Json
        if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'mihomo-traffic' -and
            $message.profile.test -and $message.executable) {
            $message.executable
        }
    } | Select-Object -Last 1
    if (-not $testExecutable -or -not (Test-Path -LiteralPath $testExecutable -PathType Leaf)) {
        throw 'Cargo did not report the application test executable'
    }
    $listing = @(& $testExecutable --list --ignored)
    if ($LASTEXITCODE -ne 0) { throw 'Failed to list desktop tests' }
    $testNames = @($listing | ForEach-Object {
        if ($_ -match '^(.+): test$') { $Matches[1] }
    } | Where-Object {
        if ($Suite -eq 'Ui') {
            $_ -match '^ui::.*::gpui_'
        } else {
            $_.StartsWith('taskbar_probe::') -or $_.StartsWith('native::taskbar::positioning_tests::') -or
                $_.StartsWith('tray::tests::')
        }
    } | Where-Object { -not $Filter -or $_.Contains($Filter) } | Sort-Object)
    if ($testNames.Count -eq 0) { throw "No ignored tests matched suite '$Suite' and filter '$Filter'" }
    if ($List) { $testNames; return }

    if ($testNames -match '^ui::desktop_host::tests::gpui_') {
        $interfaceArguments = @('build', '--locked', '--message-format=json-render-diagnostics')
        if ($Release) { $interfaceArguments += '--release' }
        $interfaceMessages = @(cargo @interfaceArguments)
        if ($LASTEXITCODE -ne 0) { throw 'Desktop interface build failed' }
        $env:MIHOMO_DESKTOP_TEST_EXE = $interfaceMessages | ForEach-Object {
            $message = $_ | ConvertFrom-Json
            if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'mihomo-traffic' -and
                -not $message.profile.test -and $message.executable) {
                $message.executable
            }
        } | Select-Object -Last 1
        if (-not $env:MIHOMO_DESKTOP_TEST_EXE) { throw 'Cargo did not report the interface executable' }
    }

    Set-WindowsApplicationManifest -ExecutablePath $testExecutable -ManifestPath (Join-Path $projectRoot 'assets/app.manifest')
    $logDirectory = Join-Path $projectRoot ('target/desktop-tests/' + $Suite.ToLowerInvariant())
    New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
    foreach ($testName in $testNames) {
        Write-Output "Running $testName"
        $logBase = Join-Path $logDirectory $testName.Replace('::', '-')
        # Each GPUI test owns a native event loop and starts in a fresh process.
        $testProcess = Start-Process -FilePath $testExecutable -ArgumentList @($testName, '--exact', '--ignored', '--test-threads=1', '--nocapture') `
            -WindowStyle Hidden -PassThru -Wait `
            -RedirectStandardOutput "$logBase.out" -RedirectStandardError "$logBase.err"
        Get-Content -LiteralPath "$logBase.out"
        Get-Content -LiteralPath "$logBase.err"
        if ($testProcess.ExitCode -ne 0) { throw "Desktop test failed: $testName (exit $($testProcess.ExitCode))" }
    }
} finally {
    $env:MIHOMO_DESKTOP_TEST_EXE = $oldInterfaceExecutable
    Pop-Location
}
