//! Singer Pro core: pure business foundations, independent of Tauri and the UI.

pub mod accounting;
pub mod audit;
pub mod auth;
pub mod authz;
pub mod backup;
pub mod catalogue;
pub mod customers;
pub mod db;
pub mod error;
pub mod ids;
pub mod installments;
pub mod inventory;
pub mod money;
pub mod pricing;
pub mod purchases;
pub mod recovery;
pub mod sales;
pub mod sequence;
pub mod settings;
pub mod stock;
pub mod suppliers;
pub mod time;
pub mod users;

#[cfg(test)]
mod p1_tests;
#[cfg(test)]
mod p2_tests;
#[cfg(test)]
mod p3_tests;
#[cfg(test)]
mod p4_tests;

pub use error::{CoreError, CoreResult};
pub use ids::Id;
pub use money::Money;
