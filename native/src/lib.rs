//! Native companion app for the WoW chat translator addon.

pub mod core;
pub mod secret;
pub mod strip;
pub mod strip_reader;
pub mod translators;
#[cfg(windows)]
pub mod capture;
