#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
. "$HOME/.local/state/truhabit/env.sh"
cd "$HOME/projects/truhabit"
bash scripts/wsl/database.sh
if [[ ${1:-} != --skip-build ]]; then
  (cd web; npm ci; npm run build)
  (cd prototype-chain; npm ci)
  cargo build --workspace --locked
fi
export APP_ENV=development SITE_ORIGIN=http://127.0.0.1:8788 BIND_ADDR=127.0.0.1:8788
unset TRUSTED_PROXY_IP
export DATABASE_URL PGPASSWORD
PGPASSWORD=$(jq -r .owner_password .local/database.json)
DATABASE_URL="postgres://truhabit_owner:$PGPASSWORD@127.0.0.1:55433/truhabit"
target/debug/truhabit-api migrate
/usr/lib/postgresql/16/bin/psql -h 127.0.0.1 -p 55433 -U truhabit_owner -d truhabit -v ON_ERROR_STOP=1 <<'SQL'
GRANT CONNECT ON DATABASE truhabit TO truhabit_app;
GRANT USAGE ON SCHEMA public TO truhabit_app;
GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO truhabit_app;
GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO truhabit_app;
REVOKE ALL ON TABLE _sqlx_migrations FROM truhabit_app;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
SQL
DATABASE_URL="postgres://truhabit_app:$(jq -r .app_password .local/database.json)@127.0.0.1:55433/truhabit"
unset PGPASSWORD
echo 'WSL application: http://127.0.0.1:8788 (independent local database).'
exec target/debug/truhabit-api
