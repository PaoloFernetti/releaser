//! releaser-rs: semantic-release для Rust-workspace.
//!
//! Conventional Commits → semver bump → `Cargo.toml` → `CHANGELOG.md` →
//! commit + annotated tag. Публикация (`cargo publish`), `cargo-semver-checks`
//! и GitHub Release — opt-in; без флагов инструмент пишет только версию,
//! changelog и тег.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod bump;
pub mod changelog;
pub mod commit;
pub mod config;
pub mod error;
pub mod git;
pub mod manifest;
pub mod release;

pub use bump::{decide_bump, next_version, BumpPlan};
pub use changelog::{insert_entry, render_entry};
pub use commit::{parse_commit, ConventionalCommit, RawCommit};
pub use config::{Bump, CommitTypeConfig, Locale, PackageMode, ReleaseConfig};
pub use error::{ReleaseError, Result};
pub use git::Git;
pub use manifest::{publish_order, read_version, update_path_dependencies, write_version};
pub use release::{apply, preview, Options, Outcome, Preview};
