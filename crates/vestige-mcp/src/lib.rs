//! Vestige MCP Server Library
//!
//! Shared modules accessible to all binaries in the crate.
//!
//! Every module that touches the legacy SQLite store (`Storage`) is
//! quarantined behind the `legacy-sqlite` feature (build/t5-legacy-isolation,
//! mirroring vestige-core). That feature is not a default: the 4.0 binaries
//! do not link rusqlite. Opt in with `--features legacy-sqlite`.

#[cfg(feature = "legacy-sqlite")]
pub mod actor_surface;

#[cfg(feature = "legacy-sqlite")]
pub mod autopilot;
#[cfg(feature = "legacy-sqlite")]
pub mod cognitive;
#[cfg(feature = "legacy-sqlite")]
pub mod dashboard;
#[cfg(feature = "legacy-sqlite")]
pub mod protocol;
#[cfg(feature = "legacy-sqlite")]
pub mod resources;
#[cfg(feature = "legacy-sqlite")]
pub mod server;
#[cfg(feature = "legacy-sqlite")]
pub mod tools;
#[cfg(feature = "legacy-sqlite")]
pub mod trace_recorder;

/// Whether this binary was compiled with an embedding runtime and a vector
/// index at all. Builds without them (the Android/Termux profile, #145) are
/// valid builds, and every status surface must say "built without embeddings"
/// where it would otherwise look like a runtime that failed to start.
pub const fn embeddings_compiled_in() -> bool {
    cfg!(all(feature = "embeddings", feature = "vector-search"))
}
