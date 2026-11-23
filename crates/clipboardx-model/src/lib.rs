#![forbid(unsafe_code)]

//! Single Source of Truth (SSOT) contract layer for ClipboardX.
//!
//! Centralizes domain models, strongly typed newtype IDs, theme semantic tokens,
//! pixel layout constants, shortcut actions, application settings, standardized
//! domain errors, and multi-backend port traits.

pub mod clip;
pub mod error;
pub mod ports;
pub mod settings;
pub mod shortcuts;
pub mod theme;

pub use clip::*;
pub use error::*;
pub use ports::*;
pub use settings::*;
pub use shortcuts::*;
pub use theme::*;
