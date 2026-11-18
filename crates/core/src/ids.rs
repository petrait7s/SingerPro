use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Portable, time-ordered primary key (UUIDv7), stored as TEXT.
/// Safe to migrate to PostgreSQL `uuid` unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(Uuid);

impl Id {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn parse(s: &str) -> Result<Self, crate::CoreError> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|_| crate::CoreError::Validation(format!("invalid id: {s}")))
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_time_ordered() {
        let a = Id::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = Id::new();
        assert_ne!(a, b);
        assert!(a < b, "UUIDv7 must sort by creation time");
    }

    #[test]
    fn roundtrip() {
        let a = Id::new();
        assert_eq!(Id::parse(&a.to_string()).unwrap(), a);
        assert!(Id::parse("nope").is_err());
    }
}
