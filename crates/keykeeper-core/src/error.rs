use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("this file is not a KeyKeeper vault")]
    NotAVault,
    #[error("vault format version {0} is not supported by this version of KeyKeeper")]
    UnsupportedFormat(u16),
    #[error(
        "vault data schema {0} is newer than this version of KeyKeeper supports; please update"
    )]
    UnsupportedSchema(u32),
    #[error("the vault's key-derivation parameters are invalid")]
    InvalidKdfParams,
    #[error("wrong password")]
    BadPassword,
    #[error("the vault is corrupted or has been tampered with")]
    Corrupt,
    #[error("the vault contents could not be read: {0}")]
    Data(String),
    #[error("a file already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("password must not be empty")]
    EmptyPassword,
    #[error("the system random number generator failed")]
    Rng,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RepoError {
    #[error("no key with id {0}")]
    KeyNotFound(uuid::Uuid),
    #[error("no trade with id {0}")]
    TradeNotFound(uuid::Uuid),
}
