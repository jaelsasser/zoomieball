#![warn(missing_docs)]

//! Allocation-stable Zoomie population adapter for Zoomieball.

mod backend;
mod checkpoint;
mod encode;
#[cfg(test)]
mod fixture;
mod pool;
mod witness;

pub use backend::ZoomieBackend;
