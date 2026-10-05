param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$cargoExe = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
if (-not (Test-Path -LiteralPath $cargoExe)) {
    $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargoCommand) { throw 'Install native Windows MSVC Rust from https://rustup.rs/ first.' }
    $cargoExe = $cargoCommand.Source
}
$originalLocation = Get-Location
try {
    Set-Location -LiteralPath $truhabitRoot
    & (Join-Path $PSScriptRoot 'setup-postgres.ps1')
    if (-not $SkipBuild) {
        Push-Location web
        try {
            if (-not (Test-Path 'node_modules')) {
                & npm.cmd ci
                if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }
            }
            & npm.cmd run build
            if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
        } finally { Pop-Location }
        & $cargoExe build --workspace --locked
        if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    }
    if (-not (Test-Path 'web/dist/index.html')) { throw 'Frontend is missing. Run without -SkipBuild.' }
    if (-not (Test-Path 'target/debug/truhabit-api.exe')) { throw 'Rust binary is missing. Run without -SkipBuild.' }
    & (Join-Path $PSScriptRoot 'migrate-local.ps1')
    $config = Get-Content -LiteralPath (Join-Path $truhabitRoot '.local/database.json') -Raw | ConvertFrom-Json
    $previousUrl = $env:DATABASE_URL
    $previousAppEnv = $env:APP_ENV
    $previousOrigin = $env:SITE_ORIGIN
    $previousBind = $env:BIND_ADDR
    $previousProxy = $env:TRUSTED_PROXY_IP
    try {
        $env:APP_ENV = 'development'
        $env:SITE_ORIGIN = 'http://127.0.0.1:8787'
        $env:BIND_ADDR = '127.0.0.1:8787'
        $env:TRUSTED_PROXY_IP = $null
        $env:DATABASE_URL = "postgres://truhabit_app:$($config.app_password)@127.0.0.1:$($config.port)/truhabit"
        Write-Host 'Open http://127.0.0.1:8787 in your browser. Stop the server with Ctrl+C.'
        & '.\target\debug\truhabit-api.exe'
        if ($LASTEXITCODE -ne 0) { throw 'Server stopped with an error. Is port 8787 already in use?' }
    } finally {
        $env:DATABASE_URL = $previousUrl
        $env:APP_ENV = $previousAppEnv
        $env:SITE_ORIGIN = $previousOrigin
        $env:BIND_ADDR = $previousBind
        $env:TRUSTED_PROXY_IP = $previousProxy
    }
} finally { Set-Location -LiteralPath $originalLocation }
