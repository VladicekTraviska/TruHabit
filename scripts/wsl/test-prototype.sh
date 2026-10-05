#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
. "$HOME/.local/state/truhabit/env.sh"
cd "$HOME/projects/truhabit"
export TEST_DATABASE_URL
TEST_DATABASE_URL="postgres://truhabit_owner:$(jq -r .owner_password .local/database.json)@127.0.0.1:55433/truhabit_test"
cargo test -p truhabit-api --test security prototype_ --locked -- --test-threads=2
cargo test -p truhabit-evidence --locked
unset TEST_DATABASE_URL
