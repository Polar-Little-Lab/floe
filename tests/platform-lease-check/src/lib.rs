//! Compiles the actual platform adapter without requiring Tauri/C toolchains.
//! This is a type-check probe, not proof of Windows runtime behavior.
use std::{fs, path::Path};
type Result<T> = std::result::Result<T, String>;
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
#[path = "../../../src-tauri/src/repository/lease.rs"]
mod lease;

pub fn probe_repository(root: &Path) -> Result<String> {
    let _lease = lease::OperationLease::acquire(root)?;
    lease::identity(&root.join(".git/index"))
}
