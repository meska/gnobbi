//! In-app update (Linux AppImage, Windows portable). One file per card; no GUI here.
pub mod verify;
pub mod startup;
#[cfg(test)]
mod startup_tests;
