//! Provider profile: config home and default account name.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT: &str = "default";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    /// Supplied by the native store, never accepted from persisted settings.
    #[serde(skip)]
    pub root: Option<PathBuf>,
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
