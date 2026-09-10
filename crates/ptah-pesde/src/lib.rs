//! Pesde-backed package management for ptah projects: the embedded
//! engine behind `ptah package add|remove|install|update`.
//!
//! An I/O adapter in the `ptah-config` pattern — composed from
//! `ptah-cli`, never seen by `ptah-core`. Every pesde type stays behind
//! this crate's API: pesde is pinned exactly (`=0.7.4`) with no
//! library-stability promise, so an upgrade is deliberately a
//! one-crate change (the CLI-layer behaviors pesde does not ship as a
//! library — manifest edits, the `--locked` staleness check, lockfile
//! writing — are re-implemented here over public types).
//!
//! Project layout (all inside the project's `.ptah/`): `pesde.toml`
//! manifest (user-owned, committed), `pesde.lock` lockfile
//! (generated, committed), `luau_packages/` (generated, ignored), and
//! the `.pesde/` cache (generated, ignored). Per-project caches under
//! `.ptah/.pesde/` keep ptah invisible to a user's own pesde install
//! and put the content-addressable store on the same filesystem as the
//! packages it hard-links into.

pub mod driver;
pub mod error;
pub mod luaurc;
pub mod manifest;
pub mod project;

#[cfg(feature = "test-fixtures")]
pub mod fixtures;

pub use error::Error;
pub use project::PackageProject;

/// The registry index used when the manifest declares no `[indices]`
/// table: pesde's own default index, injected in-memory at resolve
/// time (the manifest itself never grows an `[indices]` table ptah did
/// not write — user entries survive byte-for-byte). `PTAH_DEFAULT_INDEX`
/// overrides the compiled-in URL — the air-gapped escape hatch, and
/// how the offline test suite points the default at a loopback
/// fixture.
pub const DEFAULT_INDEX_URL: &str = "https://github.com/pesde-pkg/index";

/// Resolve the default index URL (env override > compiled-in const).
pub fn default_index_url() -> String {
    std::env::var("PTAH_DEFAULT_INDEX").unwrap_or_else(|_| DEFAULT_INDEX_URL.to_string())
}
