#![warn(rust_2018_idioms)]
#![warn(missing_docs)]
#![allow(dead_code)]

pub mod config;
pub mod conn;
mod error;
pub mod message;

pub use error::Error;
