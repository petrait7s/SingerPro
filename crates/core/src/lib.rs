//! Singer Pro core: pure business foundations, independent of Tauri and the UI.

pub mod audit;
pub mod db;
pub mod error;
pub mod ids;
pub mod money;
pub mod sequence;
pub mod time;

pub use error::{CoreError, CoreResult};
pub use ids::Id;
pub use money::Money;
