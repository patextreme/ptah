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
/// fixture. A value that does not parse fails the operations that
/// would consult the default index as a usage error
/// ([`parse_default_index_url`]).
pub const DEFAULT_INDEX_URL: &str = "https://github.com/pesde-pkg/index";

/// Resolve the default index URL (env override > compiled-in const).
pub fn default_index_url() -> String {
    std::env::var("PTAH_DEFAULT_INDEX").unwrap_or_else(|_| DEFAULT_INDEX_URL.to_string())
}

/// Parse a default-index URL value, mapping a malformed one to a
/// usage-class error naming the override mechanism. The compiled-in
/// const parses by construction, so failures in practice mean a
/// malformed `PTAH_DEFAULT_INDEX` value — the operations that would
/// consult the default index fail here, with a diagnostic naming the
/// env var, before anything touches the manifest.
pub fn parse_default_index_url(url: &str) -> Result<gix::Url, Error> {
    gix::Url::try_from(url).map_err(|e| Error::InvalidDefaultIndex {
        source: format!("`{url}`: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_index_url_validation_matches_gix() {
        // The compiled-in default and a file:// override parse.
        let url = parse_default_index_url(DEFAULT_INDEX_URL).unwrap();
        assert!(url.to_bstring().to_string().contains("pesde-pkg/index"));
        assert!(parse_default_index_url("file:///tmp/index").is_ok());

        // A malformed override is a usage-class error (exit 2) naming
        // the value and the env var.
        let err = parse_default_index_url("ht tp://x").unwrap_err();
        assert!(matches!(err, Error::InvalidDefaultIndex { .. }), "{err}");
        assert!(err.is_usage());
        let msg = err.to_string();
        assert!(msg.contains("PTAH_DEFAULT_INDEX"), "{msg}");
        assert!(msg.contains("ht tp://x"), "{msg}");
    }
}
