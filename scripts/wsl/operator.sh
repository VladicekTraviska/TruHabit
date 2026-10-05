#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
cd "$HOME/projects/truhabit"
email=${1:?Existing account email required}
action=${2:?Use grant or revoke}
[[ "$action" == grant || "$action" == revoke ]] || exit 1
export PGPASSWORD
PGPASSWORD=$(jq -r .owner_password .local/database.json)
if [[ "$action" == grant ]]; then
  /usr/lib/postgresql/16/bin/psql -h 127.0.0.1 -p 55433 -U truhabit_owner -d truhabit \
    -v ON_ERROR_STOP=1 --set=operator_email="$email" <<'SQL'
INSERT INTO prototype_operators(user_id)
SELECT id FROM users WHERE lower(email)=lower(:'operator_email')
ON CONFLICT DO NOTHING;
SELECT count(*) AS matching_operator_accounts FROM prototype_operators o JOIN users u ON u.id=o.user_id WHERE lower(u.email)=lower(:'operator_email');
SQL
else
  /usr/lib/postgresql/16/bin/psql -h 127.0.0.1 -p 55433 -U truhabit_owner -d truhabit \
    -v ON_ERROR_STOP=1 --set=operator_email="$email" <<'SQL'
DELETE FROM prototype_operators WHERE user_id IN (SELECT id FROM users WHERE lower(email)=lower(:'operator_email'));
SQL
fi
unset PGPASSWORD
