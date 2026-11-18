//! Hytera radio settings files ("codeplug") and the RCDB layout database. See `docs/file-format.md`.

pub mod container;
pub mod decode;
pub mod layout;
pub mod options;
pub mod rcdb;
pub mod structure;
pub mod value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: std::path::PathBuf, source: std::io::Error },

    #[error("{path}: XML: {source}")]
    Xml { path: std::path::PathBuf, source: roxmltree::Error },

    #[error("no RCDB file list for version {version} under {root} (looked for {tried:?})")]
    MissingVersion { version: String, root: std::path::PathBuf, tried: Vec<std::path::PathBuf> },

    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;
