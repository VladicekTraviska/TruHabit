# TruHabit

A web application prototype for personal running commitments and voluntary, employer-funded activity rewards. English is the default language; Czech is also supported.

| Mode | Workflow | Funds |
| --- | --- | --- |
| Personal challenges (B2C) | Set a goal → activate a stake → upload GPX/FIT → check the recording → settle | LOCAL test credits or THT on Solana Devnet |
| Team programs (B2B) | Top up a simulated company pool → choose a template → publish → invite participants → check runs → automatically award points | Separate corporate benefit points; no Phantom. Employer Match uses an explicitly accepted pledge of already earned corporate points |
| Personal plans | Create, edit and archive goals | Planning only; no deposit |

THT is a custom test token, not USDC. The prototype does not process real payments, use mainnet or connect to live Garmin/Strava APIs. Manual activity files are checked for consistency and goal eligibility; they do not authenticate the runner.

## Run from source

On Windows x64, install Node.js **24.14.0**, Visual Studio C++ Build Tools and the Rust MSVC toolchain. Rust **1.98.1** is pinned in `rust-toolchain.toml`. Run from the repository root:

```powershell
npm.cmd --prefix prototype-chain ci --ignore-scripts
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-local.ps1
```

The runner builds the frontend and API, provisions PostgreSQL **18.6** if needed, applies migrations and serves **http://127.0.0.1:8787/**. Its local database uses port **55432**. Keep the terminal open; Ctrl+C stops the API. The native database remains running and is reused on the next start.

Register an account and sign in. For personal challenges, choose **Local simulation** and add test credits before activation. For team programs, the owner tops up the simulated company point pool. To use an older recording, choose **Use a saved GPX/FIT**; a new-run challenge enforces its agreed activity window. The bundled GPX/FIT examples are synthetic.

A source checkout includes no accounts, database passwords or signing keys. Devnet deposits and oracle settlements require separately provisioned operator credentials, Phantom, internet and test tokens. Linking Phantom alone does not transfer funds.

See [Development](docs/DEVELOPMENT.md) for verification, existing Linux setup, reviewer permissions and configuration.

## How it works

React and TypeScript provide the UI. A Rust/Axum API manages accounts, permissions and workflows in PostgreSQL. Rust parses GPX/FIT, checks distance and time, compares available GPS/heart-rate/cadence signals and prevents the same qualifying recording from being credited repeatedly. Suspicious evidence can require an authorized human review.

The personal Devnet path adds a Node.js worker and an Anchor program. The stake is held in a program-controlled SPL token account. The trusted oracle authorizes success/failure settlement; cancellation and emergency timeout have separate contract conditions. The API persists signed commands before broadcast and verifies finalized transaction/account observations before recording a transfer as complete. Health data stays off-chain.

The expandable **Live process** panel displays actual observed API requests, evidence checks and blockchain operations. HTTP success alone is not proof of a finalized transfer. B2B rewards are local PostgreSQL transactions in a separate simulated point ledger. Activity Points, voluntary Events, Employer Match and a declining Monthly Budget are available; existing LOCAL team programs retain their original accounting.

## Source layout

```text
crates/             Rust API, evidence parser and domain rules
contracts/          Active Anchor prototype and SBF integration tests
prototype-chain/    Solana client, transaction worker and Node tests
web/                React UI, localization, frontend tests and synthetic fixtures
scripts/            Local startup, checks, reviewer permissions and backups
docs/               Current implementation and development reference
.github/workflows/  Source verification
```

Contract source, database migrations, tests, IDL and lockfiles are included. Historical designs, audits, presentation material, packaging utilities and private working records are preserved separately. Installed dependencies, runtime data and compiled output are excluded from source control.

## Checks

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify.ps1
npm.cmd --prefix prototype-chain test
```

The first command checks Rust formatting, Clippy, workspace tests against a separate test database, the frontend build/localization and frontend authentication/process regressions. Contract tests require installed SBF tooling; see [Solana escrow](docs/ESCROW.md). Ordinary checks do not deploy a program or send tokens.

## Reference

- [Architecture](docs/ARCHITECTURE.md)
- [Activity verification and result meanings](docs/ACTIVITY_VERIFICATION.md)
- [Team roles, rewards and privacy](docs/BUSINESS.md)
- [Devnet escrow and trust boundaries](docs/ESCROW.md)
- [Development, reviewer setup, reset and backups](docs/DEVELOPMENT.md)
- [Dependency notices and licensing status](docs/DEPENDENCIES.md)
