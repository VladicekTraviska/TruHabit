# Development

Run commands from the repository root unless another directory is specified. Keep Cargo and npm lockfiles. Private credentials, databases and build output do not belong in source control.

## Native Windows

Prerequisites: Windows x64, PowerShell, Node.js **24.14.0**, Visual Studio C++ Build Tools and Rust **1.98.1** with the MSVC toolchain. The repository pins Rust, rustfmt and Clippy.

```powershell
npm.cmd --prefix prototype-chain ci --ignore-scripts
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-local.ps1
```

The runner provisions local PostgreSQL **18.6**, creates protected credentials and separate application/test databases, builds the web/API, applies migrations and starts **http://127.0.0.1:8787**. PostgreSQL uses **55432**. Ctrl+C stops the API; the native database remains running. After a successful build, `run-local.ps1 -SkipBuild` reuses binaries and does not rebuild edited sources.

Register an account and sign in. Development mode supports local email verification without an SMTP service; authentication still applies. LOCAL personal challenges and B2B programs need no wallet or external API credentials.

## Existing Linux/WSL environment

The retained shell helpers support an already configured `truhabit` environment with its private generated tool environment and database. They do not install WSL or provision a new distribution.

From the prepared source copy, run `bash scripts/wsl/run.sh`; `--skip-build` reuses its binaries. The managed environment serves **127.0.0.1:8788**, with PostgreSQL **16** on **55433**. Its source copy and database are independent of the native Windows instance. Update that source copy before checking edited code. Never start another API on an occupied port.

## Checks

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify.ps1
npm.cmd --prefix prototype-chain test
node --test web/scripts/process.test.mjs
```

The native script checks formatting, Clippy, Rust tests and the frontend build. Rust database tests require `TEST_DATABASE_URL` and fail explicitly if it is absent. The script supplies the separate `truhabit_test` database; tests create and drop isolated schemas. Never point test commands at a live user database.

In the prepared Linux environment, `bash scripts/wsl/verify.sh` performs the retained workspace, frontend, worker and active contract checks. Contract verification requires installed SBF tooling; see [Escrow](ESCROW.md). Ordinary checks do not deploy programs or transfer tokens.

Synthetic GPX/FIT fixtures are in `web/public/prototype/` with their manifest. Real recordings and recording-specific regression material are preserved separately; ordinary source checks do not require those private inputs.

## Authorized reviewer

A reviewer needs an existing account and an explicit operator grant. Company OWNER/ADMIN roles do not grant access to private recording review.

For the native local database:

```powershell
powershell.exe -NoProfile -File .\scripts\prototype-operator.ps1 -Email 'reviewer@example.com'
# Revoke the same permission:
powershell.exe -NoProfile -File .\scripts\prototype-operator.ps1 -Email 'reviewer@example.com' -Revoke
```

For the prepared Linux database, run `bash scripts/wsl/operator.sh 'reviewer@example.com' grant`, or use `revoke`. These commands apply only to that installation's database. Grant access deliberately; reviewers can inspect submitted private evidence and make recorded decisions. A reviewer cannot accept an activity that missed the goal distance or applicable window.

## Configuration and Devnet

The API reads process environment variables; it does **not** load `.env` files automatically. `.env.example` documents available settings. Local runners supply development origin, loopback bind and database credentials.

The chain worker accepts `TRUHABIT_CHAIN_WORKER` and `TRUHABIT_NODE_BIN` as explicit installation paths, and `TRUHABIT_KEY_DIR` as an absolute private key directory. Without the latter it reads the project's private `.local/` directory. Public program/mint/wallet addresses are identifiers, not signing secrets.

The existing Devnet program has a fixed oracle. Generating a new arbitrary key does not authorize it. Deposits and success/failure settlement require the matching securely provisioned oracle or a coordinated independent deployment. Phantom users also need Devnet SOL for fees/rent and the configured THT token. LOCAL mode works independently.

Production configuration requires a public HTTPS origin, properly configured mail and database access. Prototype activity endpoints are disabled in production mode; these settings do not constitute a commercial deployment.

## Reset and backups

Under **My account → Development / Prototype**, preview the developer reset, review its impact, enter the current password and type **RESET**. The reset removes eligible prototype data while preserving the account, login, wallet link and operator permission. It never sends a chain transaction. Active/unresolved Devnet operations and financial history shared with other accounts can block it; a changed preview requires a fresh confirmation. After a connection interruption, refresh the preview before retrying.

For the native local database, `scripts/backup-local.ps1 -VerifyRestore` creates a private snapshot and verifies it in a separate temporary database. It does not overwrite the live database. Protect backups separately: they contain personal data, uploaded files and authentication records. Oracle keys require separate protected backup; do not include either in Git.

## Frontend iteration

`npm.cmd run dev` in `web/` starts Vite on **127.0.0.1:5173** with an API proxy targeting **8787**. The API must run separately. Adjust the proxy target for another API port while preserving allowed-origin settings. `npm.cmd run build` checks locale alignment, type-checks and builds `web/dist/`; English and Czech keys must remain aligned.
