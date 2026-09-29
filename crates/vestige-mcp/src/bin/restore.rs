//! Vestige restore binary.
//!
//! The SQLite restore path is compiled only with `legacy-sqlite`. The default
//! 4.0 binary does not link rusqlite.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[path = "../glibc_compat.rs"]
mod glibc_compat;

#[cfg(feature = "legacy-sqlite")]
#[path = "restore_sqlite.rs"]
mod sqlite_bin;

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "legacy-sqlite")]
    {
        sqlite_bin::main()
    }
    #[cfg(not(feature = "legacy-sqlite"))]
    {
        anyhow::bail!(
            "vestige-restore was built without legacy-sqlite. This 4.0 binary does not link SQLite."
        )
    }
}
