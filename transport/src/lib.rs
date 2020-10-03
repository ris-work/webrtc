#![warn(rust_2018_idioms)]
#![allow(dead_code)]

#[macro_use]
extern crate lazy_static;

pub mod buffer;
pub mod replay_detector;

pub use crate::buffer::Buffer;
