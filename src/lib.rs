#![cfg(unix)]

pub mod config;
pub mod engine;
pub mod files;
mod json;
pub mod mcp;
pub mod process;
pub mod report;
pub mod trace;

pub type Result<T> = std::result::Result<T, String>;

pub fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
