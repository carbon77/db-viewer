# DB Viewer

[![CI](https://github.com/carbon77/db-viewer/actions/workflows/ci.yml/badge.svg)](https://github.com/carbon77/db-viewer/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/carbon77/db-viewer)](https://github.com/carbon77/db-viewer/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

DB Viewer is a native Windows desktop application for safely exploring SQLite
files and remote PostgreSQL databases. It is written in Rust with egui and
enforces read-only access while browsing, filtering, querying, and exporting.

## Features

- Browse schema-qualified tables, partitioned tables, views, materialized views,
  indexes, triggers, columns, primary keys, foreign keys, and definitions.
- Page through large tables without loading the entire database into memory.
- Sort columns and combine filters for focused data inspection.
- Run a single read-only SQL statement with a 10,000-row display limit.
- Export complete filtered tables, views, or query results to UTF-8 CSV.
- Cancel long-running queries and exports.
- Keep multiple SQLite and PostgreSQL connections open in independent tabs.
- Switch between light and dark themes and reopen recent databases.

## Install

Download the latest files from the [Releases page](https://github.com/carbon77/db-viewer/releases/latest):

- Use the `.msi` package for a standard Windows installation with a Start menu shortcut.
- Use the `.exe` file to run the portable application without installing it.
- Use `SHA256SUMS.txt` to verify the downloaded files.

The published builds target 64-bit Windows. Initial releases are unsigned, so
Windows may display a SmartScreen warning.

## Usage

1. Start DB Viewer and select **Open**.
2. Choose **SQLite** and select a `.db`, `.sqlite`, or `.sqlite3` file, or choose
   **PostgreSQL** and enter the server, database, and account details.
3. Use **Open** or the **+** connection tab to open more databases, and switch between
   them without losing each connection's browsing, filtering, or SQL editor state.
4. Select an object in the schema sidebar to inspect its data, structure, or SQL definition.
5. Use the SQL Editor for one read-only statement at a time, or export the current table, view, or query to CSV.

SQLite databases are opened with the read-only flag. Every PostgreSQL operation
uses a separate connection and a read-only transaction, so writes (including
statements with `RETURNING`) fail even for write-capable accounts. The editor
accepts exactly one row-returning statement. Query results shown in the
application are capped at 10,000 rows; CSV export runs the full accepted query
and writes UTF-8 CSV.

The PostgreSQL form supports `Disable`, `Prefer` (the default), and `Require`
SSL modes. TLS uses the Windows system certificate store and does not require a
PostgreSQL client installation. Connection attempts have a finite timeout, and
connection errors are shown in the application.

Recent PostgreSQL connections retain only the host, port, database, username,
and SSL mode. Passwords are never written to settings and must be entered again
when reopening a recent connection. Labels are always redacted as
`user@host:port/database`. Objects from non-system PostgreSQL schemas are shown
with qualified names so same-named objects remain distinct.

## Development

Install the stable Rust toolchain, clone the repository, and run:

```powershell
cargo run
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Create an optimized portable executable with:

```powershell
cargo build --locked --release
```

The output is `target/release/rust-db-viewer.exe`.

Building the MSI locally additionally requires WiX Toolset 3.14 and cargo-wix:

```powershell
cargo install cargo-wix --version 0.3.9 --locked
cargo wix --nocapture
```

## Releases

The release workflow runs for semantic version tags such as `v0.1.0`. The tag
must match the version in `Cargo.toml`. A successful run publishes the portable
executable, MSI installer, checksums, and generated release notes.

## License

Licensed under the [MIT License](LICENSE).
