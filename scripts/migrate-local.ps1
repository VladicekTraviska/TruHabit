$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$config = Get-Content -LiteralPath (Join-Path $truhabitRoot '.local/database.json') -Raw | ConvertFrom-Json
$pgBin = Join-Path $truhabitRoot '.tools/postgresql-18.6/pgsql/bin'
$previousUrl = $env:DATABASE_URL
$previousPassword = $env:PGPASSWORD
$previousAppEnv = $env:APP_ENV
$previousOrigin = $env:SITE_ORIGIN
$previousBind = $env:BIND_ADDR
try {
    $env:APP_ENV = 'development'
    $env:SITE_ORIGIN = 'http://127.0.0.1:8787'
    $env:BIND_ADDR = '127.0.0.1:8787'
    $env:DATABASE_URL = "postgres://truhabit_owner:$($config.owner_password)@127.0.0.1:$($config.port)/truhabit"
    & (Join-Path $truhabitRoot 'target/debug/truhabit-api.exe') migrate
    if ($LASTEXITCODE -ne 0) { throw 'Migration failed.' }
    $env:PGPASSWORD = $config.owner_password
    $grantSql = 'GRANT CONNECT ON DATABASE truhabit TO truhabit_app; GRANT USAGE ON SCHEMA public TO truhabit_app; GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO truhabit_app; GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO truhabit_app; REVOKE ALL ON TABLE _sqlx_migrations FROM truhabit_app; REVOKE CREATE ON SCHEMA public FROM PUBLIC;'
    & (Join-Path $pgBin 'psql.exe') -h 127.0.0.1 -p $config.port -U truhabit_owner -d truhabit -v ON_ERROR_STOP=1 -c $grantSql
    if ($LASTEXITCODE -ne 0) { throw 'Runtime privilege setup failed.' }
} finally {
    $env:DATABASE_URL = $previousUrl
    $env:PGPASSWORD = $previousPassword
    $env:APP_ENV = $previousAppEnv
    $env:SITE_ORIGIN = $previousOrigin
    $env:BIND_ADDR = $previousBind
}
