//! App Remover: a fast Revo-Uninstaller-style program remover for Windows.
//!
//! Everything except the thin Windows wrappers in [`registry::WinRegistry`] and
//! [`system::RealSystem`] is platform independent and unit tested.

pub mod cleaner;
pub mod disk;
pub mod engine;
pub mod model;
pub mod names;
pub mod profiles;
pub mod registry;
pub mod safety;
pub mod scanner;
pub mod system;
pub mod uninstall;

#[cfg(test)]
mod tests;
