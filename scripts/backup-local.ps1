param([switch]$VerifyRestore)
$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$config = Get-Content -LiteralPath (Join-Path $truhabitRoot '.local/database.json') -Raw | ConvertFrom-Json
$pgBin = Join-Path $truhabitRoot '.tools/postgresql-18.6/pgsql/bin'
$backupDir = Join-Path $truhabitRoot '.local/backups'
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
$backup = Join-Path $backupDir ("truhabit-{0}-{1}.dump" -f (Get-Date -Format 'yyyyMMdd-HHmmss'), ([Guid]::NewGuid().ToString('N')))
$previousPassword = $env:PGPASSWORD
$temporaryDatabase = 'truhabit_restore_' + [Guid]::NewGuid().ToString('N') + '_test'
$created = $false
try {
    $env:PGPASSWORD = $config.owner_password
    & (Join-Path $pgBin 'pg_dump.exe') -h 127.0.0.1 -p $config.port -U truhabit_owner -d truhabit --format=custom --no-owner --no-privileges --file=$backup
    if ($LASTEXITCODE -ne 0) { throw 'Backup failed. Any incomplete archive must not be used.' }
    & (Join-Path $pgBin 'pg_restore.exe') --list $backup | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Backup archive inspection failed.' }
    $report = [ordered]@{ created_utc = [DateTime]::UtcNow.ToString('o'); file = [IO.Path]::GetFileName($backup); sha256 = (Get-FileHash -LiteralPath $backup -Algorithm SHA256).Hash; restore_verified = $false }
    if ($VerifyRestore) {
        # A fresh random database is the ONLY restore destination. Never overwrite the application DB.
        if ($temporaryDatabase -notmatch '^truhabit_restore_[a-f0-9]{32}_test$') { throw 'Unsafe restore database name.' }
        & (Join-Path $pgBin 'createdb.exe') -h 127.0.0.1 -p $config.port -U truhabit_owner --owner=truhabit_owner $temporaryDatabase
        if ($LASTEXITCODE -ne 0) { throw 'Cannot create isolated restore database.' }
        $created = $true
        & (Join-Path $pgBin 'pg_restore.exe') -h 127.0.0.1 -p $config.port -U truhabit_owner -d $temporaryDatabase --single-transaction --exit-on-error --no-owner --no-privileges $backup
        if ($LASTEXITCODE -ne 0) { throw 'Restore verification failed.' }
        $check = "SELECT json_build_object('users',(SELECT count(*) FROM users),'goals',(SELECT count(*) FROM goals),'events',(SELECT count(*) FROM goal_events),'prototype_challenges',(SELECT count(*) FROM prototype_challenges),'prototype_uploads',(SELECT count(*) FROM prototype_uploads),'prototype_commands',(SELECT count(*) FROM prototype_commands),'organizations',(SELECT count(*) FROM organizations),'company_programs',(SELECT count(*) FROM company_programs),'migrations',(SELECT count(*) FROM _sqlx_migrations WHERE success),'invalid_constraints',(SELECT count(*) FROM pg_constraint WHERE connamespace='public'::regnamespace AND NOT convalidated))"
        $counts = & (Join-Path $pgBin 'psql.exe') -h 127.0.0.1 -p $config.port -U truhabit_owner -d $temporaryDatabase -X -t -A -v ON_ERROR_STOP=1 -c $check
        if ($LASTEXITCODE -ne 0) { throw 'Restored data checks failed.' }
        $summary = ($counts -join '') | ConvertFrom-Json
        if ($summary.migrations -lt 1 -or $summary.invalid_constraints -ne 0) { throw 'Restored schema is incomplete.' }
        $report.restore_verified = $true
        $report.restored_counts = $summary
    }
    $report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath ($backup + '.json') -Encoding UTF8
    Write-Host "Backup saved: $backup"
    Write-Host "Restore verified: $($report.restore_verified)"
} finally {
    if ($created) {
        if ($temporaryDatabase -notmatch '^truhabit_restore_[a-f0-9]{32}_test$') { throw 'Refusing unsafe database cleanup.' }
        & (Join-Path $pgBin 'dropdb.exe') -h 127.0.0.1 -p $config.port -U truhabit_owner $temporaryDatabase
        if ($LASTEXITCODE -ne 0) { Write-Warning "Remove isolated restore database manually: $temporaryDatabase" }
    }
    $env:PGPASSWORD = $previousPassword
}
