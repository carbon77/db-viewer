# DB Viewer

[![CI](https://github.com/carbon77/db-viewer/actions/workflows/ci.yml/badge.svg)](https://github.com/carbon77/db-viewer/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/carbon77/db-viewer)](https://github.com/carbon77/db-viewer/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

DB Viewer is a native Windows desktop application for safely exploring SQLite
databases. It is written in Rust with egui and opens databases in read-only
mode, so browsing, filtering, querying, and exporting cannot modify the source
file.

## Features

- Browse tables, views, indexes, triggers, columns, foreign keys, and creation SQL.
- Page through large tables without loading the entire database into memory.
- Sort columns and combine filters for focused data inspection.
- Run a single read-only SQL statement with a 10,000-row display limit.
- Export complete filtered tables, views, or query results to UTF-8 CSV.
- Cancel long-running queries and exports.
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
2. Choose a `.db`, `.sqlite`, or `.sqlite3` file.
3. Select an object in the schema sidebar to inspect its data, structure, or SQL definition.
4. Use the SQL Editor for one read-only statement at a time, or export the current table, view, or query to CSV.

Databases are opened with SQLite's read-only flag, and write-capable SQL is
rejected. Query results shown in the application are capped at 10,000 rows;
CSV export runs the full accepted query.

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
