$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$toolsDir = Join-Path $truhabitRoot '.tools'
$destination = Join-Path $toolsDir 'postgresql-18.6'
$archive = Join-Path $toolsDir 'postgresql-18.6-4-windows-x64-binaries.zip'
$expectedHash = '1DF55002AFE95B945D934C078B13E82C1603FA546731E511D068AA983B4EAD28'
if (Test-Path -LiteralPath (Join-Path $destination 'pgsql/bin/initdb.exe')) {
    Write-Host 'PostgreSQL binaries already present.'
    exit 0
}
if (Test-Path -LiteralPath $destination) { throw 'Partial installation exists. Inspect .tools/postgresql-18.6 before retrying.' }
New-Item -ItemType Directory -Force -Path $toolsDir | Out-Null
if (-not (Test-Path -LiteralPath $archive)) {
    Write-Host 'Downloading official PostgreSQL 18.6 Windows x64 binaries (about 383 MB).'
    Invoke-WebRequest -UseBasicParsing -Uri 'https://get.enterprisedb.com/postgresql/postgresql-18.6-4-windows-x64-binaries.zip' -OutFile $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expectedHash) {
    throw 'PostgreSQL archive hash mismatch. Installation stopped; inspect the download.'
}
Expand-Archive -LiteralPath $archive -DestinationPath $destination
& (Join-Path $destination 'pgsql/bin/postgres.exe') --version
if ($LASTEXITCODE -ne 0) { throw 'PostgreSQL executable check failed.' }
