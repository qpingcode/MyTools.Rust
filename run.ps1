param([switch]$Release, [switch]$BuildOnly, [switch]$Show)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    $buildArgs = @('build', '--workspace', '--locked')
    $profile = 'debug'
    if ($Release) {
        $buildArgs += '--release'
        $profile = 'release'
    }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Cargo 编译失败' }

    $output = Join-Path $PSScriptRoot "target/$profile"
    $licenses = Join-Path $output 'licenses'
    New-Item -ItemType Directory -Force -Path $licenses | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'ui/assets/icons/LICENSE') -Destination (Join-Path $licenses 'Lucide.txt')
    if (-not (Test-Path -LiteralPath (Join-Path $output 'config.json'))) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'config.example.json') -Destination (Join-Path $output 'config.json')
    }
    foreach ($plugin in @('calculator', 'applications', 'clipboard', 'chrome-bookmark', 'encode', 'decode', 'formatter', 'process-killer', 'uuid-generator', 'translator', 'quick-text')) {
        $pluginDirectory = Join-Path $output "plugins/$plugin"
        New-Item -ItemType Directory -Force -Path $pluginDirectory | Out-Null
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot "plugins/$plugin/plugin.json") -Destination $pluginDirectory
        $localeDirectory = Join-Path $pluginDirectory 'locales'
        New-Item -ItemType Directory -Force -Path $localeDirectory | Out-Null
        Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot "plugins/$plugin/locales") -Filter '*.json' |
            Copy-Item -Destination $localeDirectory
        Copy-Item -LiteralPath (Join-Path $output "mytools-$plugin.exe") -Destination $pluginDirectory
        $example = Join-Path $PSScriptRoot "plugins/$plugin/config.example.json"
        if (Test-Path -LiteralPath $example) {
            Copy-Item -LiteralPath $example -Destination $pluginDirectory
        }
    }
    if (-not $BuildOnly) {
        $startArguments = @{}
        if ($Show) { $startArguments.ArgumentList = '--show' }
        Start-Process -FilePath (Join-Path $output 'mytools.exe') -WorkingDirectory $output -WindowStyle Normal @startArguments
    }
} finally {
    Pop-Location
}
