#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
[[ $(id -un) == truhabit ]] || { echo 'Run as truhabit.' >&2; exit 1; }
project="$HOME/projects/truhabit"
pgbin=/usr/lib/postgresql/16/bin
config="$project/.local/database.json"
pgdata="$project/data/postgres-wsl"
mkdir -p "$project/.local" "$project/data"
chmod 700 "$project/.local"
if [[ ! -f "$config" ]]; then
  [[ ! -e "$pgdata" ]] || { echo 'Database exists without credentials; refusing to overwrite.' >&2; exit 1; }
  owner_password=$(openssl rand -hex 32)
  app_password=$(openssl rand -hex 32)
  jq -n --arg owner "$owner_password" --arg app "$app_password" '{owner_password:$owner, app_password:$app, port:55433}' > "$config"
  password_file=$(mktemp "$project/.local/init-password.XXXXXX")
  trap 'rm -f -- "$password_file"' EXIT
  printf '%s\n' "$owner_password" > "$password_file"
  "$pgbin/initdb" -D "$pgdata" -U truhabit_owner --pwfile="$password_file" -A scram-sha-256 --encoding=UTF8 --locale=C
  rm -f -- "$password_file"
  trap - EXIT
  cat > "$pgdata/postgresql.auto.conf" <<'CONF'
listen_addresses = '127.0.0.1'
unix_socket_directories = ''
port = 55433
max_connections = 50
shared_buffers = '128MB'
password_encryption = 'scram-sha-256'
log_statement = 'none'
log_min_error_statement = 'panic'
log_parameter_max_length_on_error = 0
CONF
fi
# This user-owned cluster uses loopback TCP. Ubuntu's system socket directory
# belongs to postgres and is intentionally inaccessible to the development user.
# Repair clusters initialized before this setting was added without resetting data.
if ! grep -Eq '^[[:space:]]*unix_socket_directories[[:space:]]*=' "$pgdata/postgresql.auto.conf"; then
  printf "unix_socket_directories = ''\n" >> "$pgdata/postgresql.auto.conf"
fi
if ! "$pgbin/pg_ctl" status -D "$pgdata" >/dev/null; then
  "$pgbin/pg_ctl" start -D "$pgdata" -l "$project/data/postgres-wsl.log" -w
fi
export PGPASSWORD
PGPASSWORD=$(jq -r .owner_password "$config")
app_password=$(jq -r .app_password "$config")
# Secrets go through stdin, never command arguments, logs, or shell tracing.
"$pgbin/psql" -h 127.0.0.1 -p 55433 -U truhabit_owner -d postgres -v ON_ERROR_STOP=1 <<SQL
SELECT 'CREATE ROLE truhabit_app LOGIN PASSWORD ''$app_password''' WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname='truhabit_app')\gexec
SELECT 'CREATE DATABASE truhabit OWNER truhabit_owner' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname='truhabit')\gexec
SELECT 'CREATE DATABASE truhabit_test OWNER truhabit_owner' WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname='truhabit_test')\gexec
SQL
unset PGPASSWORD app_password owner_password
echo 'Independent WSL PostgreSQL database ready on 127.0.0.1:55433.'
