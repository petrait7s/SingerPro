//! Singer Pro core: pure business foundations, independent of Tauri and the UI.

pub mod audit;
pub mod auth;
pub mod authz;
pub mod backup;
pub mod db;
pub mod error;
pub mod ids;
pub mod money;
pub mod recovery;
pub mod sequence;
pub mod settings;
pub mod time;
pub mod users;

#[cfg(test)]
mod p1_tests;

pub use error::{CoreError, CoreResult};
pub use ids::Id;
pub use money::Money;
