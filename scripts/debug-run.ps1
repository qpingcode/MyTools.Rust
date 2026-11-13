# Builds the workspace and runs MyTools from target/, isolated from the installed copy
# (its own config.json and window position). Defaults to a fast debug build.
param([switch]$Release, [switch]$BuildOnly, [switch]$Show)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'dev-environment.ps1')
Push-Location $root
try {
    Enter-DeveloperEnvironment
    $buildArgs = @('build', '--workspace', '--locked')
    $profile = 'debug'
    if ($Release) {
        $buildArgs += '--release'
        $profile = 'release'
    }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Cargo 编译失败' }

    $output = Join-Path $root "target/$profile"
    $licenses = Join-Path $output 'licenses'
    New-Item -ItemType Directory -Force -Path $licenses | Out-Null
    Copy-Item -LiteralPath (Join-Path $root 'ui/assets/icons/LICENSE') -Destination (Join-Path $licenses 'Lucide.txt')
    if (-not (Test-Path -LiteralPath (Join-Path $output 'config.json'))) {
        Copy-Item -LiteralPath (Join-Path $root 'config.example.json') -Destination (Join-Path $output 'config.json')
    }
    foreach ($manifest in Get-ChildItem -Path (Join-Path $root 'plugins/*/plugin.json')) {
        $plugin = $manifest.Directory.Name
        $entry = (Get-Content -LiteralPath $manifest.FullName -Raw | ConvertFrom-Json).entry
        $pluginDirectory = Join-Path $output "plugins/$plugin"
        New-Item -ItemType Directory -Force -Path $pluginDirectory | Out-Null
        Copy-Item -LiteralPath (Join-Path $root "plugins/$plugin/plugin.json") -Destination $pluginDirectory
        $localeDirectory = Join-Path $pluginDirectory 'locales'
        New-Item -ItemType Directory -Force -Path $localeDirectory | Out-Null
        Get-ChildItem -LiteralPath (Join-Path $root "plugins/$plugin/locales") -Filter '*.json' |
            Copy-Item -Destination $localeDirectory
        Copy-Item -LiteralPath (Join-Path $output "$entry.exe") -Destination $pluginDirectory
        Get-ChildItem -Path (Join-Path $root "plugins/$plugin") -Filter '*.example.json' |
            Copy-Item -Destination $pluginDirectory
    }
    if (-not $BuildOnly) {
        $startArguments = @{}
        if ($Show) { $startArguments.ArgumentList = '--show' }
        Start-Process -FilePath (Join-Path $output 'mytools.exe') -WorkingDirectory $output -WindowStyle Normal @startArguments
    }
} finally {
    Pop-Location
}
