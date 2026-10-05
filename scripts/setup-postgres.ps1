$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$pgBin = Join-Path $truhabitRoot '.tools/postgresql-18.6/pgsql/bin'
$localConfig = Join-Path $truhabitRoot '.local/database.json'
$pgData = Join-Path $truhabitRoot 'data/postgres'
if (-not (Test-Path (Join-Path $pgBin 'initdb.exe'))) { & (Join-Path $PSScriptRoot 'install-postgres.ps1') }
if (-not (Test-Path -LiteralPath $localConfig)) {
    if (Test-Path -LiteralPath $pgData) { throw 'Existing PostgreSQL data without credentials. Refusing to overwrite.' }
    $localDir = Join-Path $truhabitRoot '.local'
    New-Item -ItemType Directory -Force -Path $localDir | Out-Null
    $account = "$env:USERDOMAIN\$env:USERNAME"
    & icacls.exe $localDir /inheritance:r /grant:r "${account}:(OI)(CI)F" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Cannot restrict local credentials directory.' }
    function New-LocalSecret {
        $bytes = New-Object byte[] 32
        $random = [Security.Cryptography.RandomNumberGenerator]::Create()
        try { $random.GetBytes($bytes) } finally { $random.Dispose() }
        return ([BitConverter]::ToString($bytes)).Replace('-','').ToLowerInvariant()
    }
    $config = @{ owner_password = (New-LocalSecret); app_password = (New-LocalSecret); port = 55432 }
    $config | ConvertTo-Json | Set-Content -LiteralPath $localConfig -Encoding ASCII
    $passwordFile = Join-Path $localDir 'init-password.tmp'
    $config.owner_password | Set-Content -LiteralPath $passwordFile -Encoding ASCII
    try {
        & (Join-Path $pgBin 'initdb.exe') -D $pgData -U truhabit_owner --pwfile=$passwordFile -A scram-sha-256 -E UTF8 --locale=C
        if ($LASTEXITCODE -ne 0) { throw 'PostgreSQL initialization failed.' }
    } finally { Remove-Item -LiteralPath $passwordFile -ErrorAction SilentlyContinue }
    @"
listen_addresses = '127.0.0.1'
port = 55432
max_connections = 50
shared_buffers = '128MB'
password_encryption = 'scram-sha-256'
log_statement = 'none'
log_min_error_statement = 'panic'
log_parameter_max_length_on_error = 0
"@ | Set-Content -LiteralPath (Join-Path $pgData 'postgresql.auto.conf') -Encoding ASCII
}
$config = Get-Content -LiteralPath $localConfig -Raw | ConvertFrom-Json
& (Join-Path $pgBin 'pg_ctl.exe') status -D $pgData | Out-Null
if ($LASTEXITCODE -ne 0) {
    & (Join-Path $pgBin 'pg_ctl.exe') start -D $pgData -l (Join-Path $truhabitRoot 'data/postgres.log') -w
    if ($LASTEXITCODE -ne 0) { throw 'Could not start local PostgreSQL.' }
}
$sqlFile = Join-Path $truhabitRoot '.local/setup-database.sql'
$previousPassword = $env:PGPASSWORD
try {
    $env:PGPASSWORD = $config.owner_password
    @"
SELECT 'CREATE ROLE truhabit_app LOGIN PASSWORD ''$($config.app_password)''' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname='truhabit_app')\gexec
SELECT 'CREATE DATABASE truhabit OWNER truhabit_owner' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname='truhabit')\gexec
SELECT 'CREATE DATABASE truhabit_test OWNER truhabit_owner' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname='truhabit_test')\gexec
"@ | Set-Content -LiteralPath $sqlFile -Encoding ASCII
    & (Join-Path $pgBin 'psql.exe') -h 127.0.0.1 -p $config.port -U truhabit_owner -d postgres -v ON_ERROR_STOP=1 -f $sqlFile
    if ($LASTEXITCODE -ne 0) { throw 'Database creation failed.' }
} finally {
    Remove-Item -LiteralPath $sqlFile -ErrorAction SilentlyContinue
    $env:PGPASSWORD = $previousPassword
}
Write-Host 'Native PostgreSQL is available on 127.0.0.1:55432. Credentials were not printed.'
