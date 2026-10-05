# Architecture

TruHabit is a local web prototype for personal running commitments and employer-funded running rewards. LOCAL credits and Solana Devnet test tokens are separate modes; neither represents real money.

## Components

| Component | Implementation | Responsibility |
| --- | --- | --- |
| Web application | React, TypeScript, Vite; `web/` | Accounts, challenges, company programs, uploads, reviews, Phantom interactions and presentation console |
| HTTP API | Rust, Axum, SQLx; `crates/api/` | Authentication, permissions, validation, transactions, private evidence and settlement orchestration |
| Database | PostgreSQL; `crates/api/migrations/` | Accounts, sessions, participation, private uploads, local credit records and application events |
| Activity parser | Rust; `crates/evidence/` | Bounded GPX/FIT parsing, telemetry summaries, fingerprints and consistency checks |
| Domain rules | Rust; `crates/rules/` | Goal and commitment models used by the API |
| Chain worker | Node.js, `@solana/web3.js`; `prototype-chain/` | Devnet checks, transaction preparation, signature validation, broadcast and reconciliation |
| Devnet program | Rust, Anchor, SPL Token; `contracts/prototype/` | Token custody, fixed terms and authorized settlement |
| Contract tests | `contracts/prototype-tests/` | Local SBF transaction tests and prototype IDL generation |

The Rust API serves the built frontend from `web/dist/`. PostgreSQL and the API run locally. Solana Devnet and Phantom are required only for the personal Devnet path.

## Personal commitments

Creating a challenge records its target, mode, stake and deadlines. Activation is separate: LOCAL reserves credits in the database; Devnet transfers THT to a program-controlled vault after the required signatures. Uploading a GPX/FIT file records evidence and eligibility; it does not itself move tokens.

An eligible, accepted recording makes success settlement available. Suspicious evidence needs an authorized review first. Rejected evidence can be followed by another eligible recording before the upload deadline. Failure settlement, cancellation before the start and the later emergency refund have distinct conditions.

THT is the six-decimal **TruHabit Test Token**, not USDC. Challenge metadata lives in a program account; the tokens live in a separate SPL token vault controlled by the commitment PDA.

## Chain trust

The contract fixes the mint and recipient and stores the owner, amount, deadline terms and terms hash. Deposits require owner and oracle signatures. Success and failure require the oracle. Owner cancellation and the emergency timeout do not require the oracle's secret.

The oracle is a trusted off-chain service. Solana verifies signatures, account constraints, timing and token movement. Rust evaluates the activity. The chain does not inspect Garmin telemetry, authenticate the runner or independently establish the truth of an uploaded recording.

The API persists a signed command before broadcast and reconciles it against finalized transaction/account observations. An RPC acknowledgement or HTTP response alone is not a completed settlement. A public checkout excludes signing keys; operating the existing fixed-oracle deployment requires separate secure provisioning. See [Escrow](ESCROW.md).

## Team programs

B2B uses employer-funded **LOCAL test credits**. Joining reserves a potential reward; accepted results allow one reward claim. Participants do not stake their own funds and do not need Phantom.

Published terms are fixed. Closing returns unused budget while protecting earned unpaid rewards and pending-review conditions. Archiving removes finished records from the active view while retaining history. Permanent deletion is limited to eligible workspaces without protected funded history. See [Business rules](BUSINESS.md).

## Evidence and privacy

Manual GPX/FIT files are editable, untrusted inputs. Checks assess consistency and goal eligibility rather than identity. Missing sensors are coverage limits, not invented measurements. Raw source files and health/GPS samples remain off-chain in PostgreSQL; employers see participation and result status rather than private recordings. Owner and explicitly authorized reviewer access is enforced by the API. Public chain transactions still expose wallet and token-transfer information.

The [verification reference](ACTIVITY_VERIFICATION.md) explains outcomes and current thresholds.

## Presentation and deployment scope

The expandable console displays actual application observations: selected API requests/results, evidence checks and available transaction signatures. Entries are bounded, ephemeral and cleared between sessions. It excludes request bodies, tokens, private files and sensitive invitation information. It is not a command-executing terminal or a blockchain node.

Live Garmin ingestion, real payments, mainnet custody, Privy, Apple sign-in and an AI runtime arbitrator are not integrated. Prototype activity routes are disabled under `APP_ENV=production`; changing that variable does not make the prototype a public commercial service.