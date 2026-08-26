mod api;
mod checksum;
mod client;
mod download;
mod error;
mod macros;
mod model;
mod path_safety;
mod utils;
mod walker;

pub const CHUNK_SIZE: usize = 1024 * 1024;

pub use client::YaShareClient;
pub use error::{Error, Result, io_error};
pub use utils::PublicKey;
