#!/usr/bin/env bash
set -Eeuo pipefail
umask 077
. "$HOME/.local/state/truhabit/env.sh"
cd "$HOME/projects/truhabit"
bash scripts/wsl/database.sh
export TEST_DATABASE_URL
TEST_DATABASE_URL="postgres://truhabit_owner:$(jq -r .owner_password .local/database.json)@127.0.0.1:55433/truhabit_test"
cargo check --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
(cd web; npm ci; npm run build)
(cd prototype-chain; npm ci; npm test)
cargo-build-sbf --manifest-path contracts/prototype/programs/truhabit-prototype/Cargo.toml --tools-version v1.57 -- --locked
cargo test --manifest-path contracts/prototype-tests/Cargo.toml --locked --test prototype -- --test-threads=1
cargo run --manifest-path contracts/prototype-tests/Cargo.toml --example build_prototype_idl --locked
unset TEST_DATABASE_URL
echo 'WSL Rust, web, prototype client, and the active prototype contract verified.'
