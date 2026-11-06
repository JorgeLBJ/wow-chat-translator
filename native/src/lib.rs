//! Native companion app for the WoW chat translator addon.

pub mod core;
pub mod strip;
pub mod strip_reader;
#[cfg(windows)]
pub mod capture;
