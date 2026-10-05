# Solana Devnet escrow

The active personal-challenge program is `contracts/prototype/`. Its SBF transaction tests and IDL builder are in `contracts/prototype-tests/`. B2B programs use LOCAL credits and do not call this program.

## Identities and custody

| Identity | Public address |
| --- | --- |
| Program | `24nGy1KAx5GSFR2C6xaHnRgKMqYyoNq3LNfu3fFtvNRa` |
| Oracle | `8m8eyZih5Mjakb8CH5Bxsi7BLp2gbderP6bKuJPm96n4` |
| THT mint | `BYa72dJf9S1sw4a4cmESd9DhinH8pQ6fy7gjbPm6VNp3` |
| Failure recipient | `6dgtm5XMrG6VTSWZGtrpZMWgZHq6mr4JuzBbjBBJTD8e` |

These are public identifiers, not private keys. THT is a six-decimal SPL test token with no monetary value, not USDC. The failure recipient is a fixed test address, not a verified charity.

`deposit` requires owner and configured oracle signatures, validates the mint and amount/time terms, creates the commitment PDA and transfers tokens into a separate SPL vault controlled by that PDA. The commitment stores the owner, amount, deadlines, identifier and terms hash; it stores no raw activity or health data.

## Terminal actions

| Action | Contract condition | Destination |
| --- | --- | --- |
| Success | Oracle signature; within recorded success/refund window | Owner |
| Failure | Oracle signature; after upload deadline and before emergency refund | Fixed recipient |
| Cancel | Owner signature; before activity start | Owner |
| Timeout | At or after emergency refund deadline, without needing the oracle | Owner |

The API adds account and evidence eligibility checks. A settled commitment remains as a replay tombstone; there is no close/reinitialize path. Mint/owner, PDA and destination constraints prevent substitution of the expected token vault or recipient.

## Off-chain trust and reconciliation

Rust checks the recording and target. The Node worker prepares and validates the authorized Devnet transaction. The API persists signed commands before broadcast and reconciles the same operation against finalized transaction/account observations. An RPC acknowledgement or timeout does not determine the final outcome; refresh/reconciliation resolves uncertain operations.

The oracle is trusted. Solana establishes authorized token movement under contract rules, not runner identity or authenticity of an uploaded file. A public source checkout excludes private keys. Existing-deployment operation needs the matching securely provisioned oracle; an independent deployment requires coordinated identities in both program and client.

## Local verification

Install the required SBF tooling separately: `cargo-build-sbf` **4.4.0** with platform-tools **v1.57**, plus the pinned host Rust toolchain. The check script does not install the full Solana development environment.

On Windows:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-contract.ps1
```

The active SBF program must be built before transaction tests so they execute current source rather than stale compiled output. In a configured Linux shell, the equivalent core commands are:

```bash
cargo-build-sbf --manifest-path contracts/prototype/programs/truhabit-prototype/Cargo.toml --tools-version v1.57 -- --locked
cargo test --manifest-path contracts/prototype-tests/Cargo.toml --locked
cargo run --manifest-path contracts/prototype-tests/Cargo.toml --example build_prototype_idl --locked
```

These checks do not deploy a program or transfer tokens on Devnet. Generated deployment keypairs and SBF build output are excluded from source control; the public prototype IDL remains in `contracts/prototype/idl/`.