# Dependencies and licensing

Rust dependencies are pinned in the root `Cargo.lock` and separate contract lockfiles. JavaScript dependencies are pinned in `web/package-lock.json` and `prototype-chain/package-lock.json`. The source repository excludes installed dependency trees and runtime binaries.

The application uses Rust/Axum/SQLx, PostgreSQL, React/TypeScript/Vite, Solana web3.js and Anchor/SPL Token. Upstream and transitive dependency license terms remain applicable. Preserve required notices when redistributing dependencies or producing executable builds.

The local PostgreSQL download is checksum-checked. A successful download, build or checksum check does not grant redistribution rights.

No project-wide license has been selected for TruHabit. Dependency licenses do not automatically license this project's original source. Select the intended project license before describing the repository as licensed open source; preserve third-party attribution.