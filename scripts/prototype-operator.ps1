param([Parameter(Mandatory=$true)][string]$Email,[switch]$Revoke)
$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$config = Get-Content -LiteralPath (Join-Path $truhabitRoot '.local/database.json') -Raw | ConvertFrom-Json
$pg = Join-Path $truhabitRoot '.tools/postgresql-18.6/pgsql/bin/psql.exe'
$previousPassword = $env:PGPASSWORD
try {
    $env:PGPASSWORD = $config.owner_password
    if ($Revoke) {
        $sql = "DELETE FROM prototype_operators WHERE user_id IN (SELECT id FROM users WHERE lower(email)=lower(:'operator_email'));"
    } else {
        $sql = "INSERT INTO prototype_operators(user_id) SELECT id FROM users WHERE lower(email)=lower(:'operator_email') ON CONFLICT DO NOTHING; SELECT count(*) AS matching_operator_accounts FROM prototype_operators o JOIN users u ON u.id=o.user_id WHERE lower(u.email)=lower(:'operator_email');"
    }
    # A psql variable quotes the email as an SQL literal; never interpolate it into SQL.
    $sql | & $pg -h 127.0.0.1 -p $config.port -U truhabit_owner -d truhabit -X -q -v ON_ERROR_STOP=1 --set="operator_email=$Email"
    if ($LASTEXITCODE -ne 0) { throw 'Operator role update failed.' }
} finally { $env:PGPASSWORD = $previousPassword }
