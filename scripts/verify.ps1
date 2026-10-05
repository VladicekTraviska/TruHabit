$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$cargoExe = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
if (-not (Test-Path -LiteralPath $cargoExe)) { $cargoExe = (Get-Command cargo -ErrorAction Stop).Source }
$originalLocation = Get-Location
try {
    Set-Location -LiteralPath $truhabitRoot
    & $cargoExe fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Rust formatting failed.' }
    & $cargoExe clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
    & (Join-Path $PSScriptRoot 'setup-postgres.ps1')
    $config = Get-Content -LiteralPath (Join-Path $truhabitRoot '.local/database.json') -Raw | ConvertFrom-Json
    $previousTestUrl = $env:TEST_DATABASE_URL
    try {
        $env:TEST_DATABASE_URL = "postgres://truhabit_owner:$($config.owner_password)@127.0.0.1:$($config.port)/truhabit_test"
        & $cargoExe test --workspace --locked
        if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
    } finally { $env:TEST_DATABASE_URL = $previousTestUrl }
    Push-Location web
    try {
        & npm.cmd ci
        if ($LASTEXITCODE -ne 0) { throw 'npm ci failed.' }
        & npm.cmd run build
        if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
    } finally { Pop-Location }
    Write-Host 'All automated checks passed.'
} finally { Set-Location -LiteralPath $originalLocation }
