//! Allowed access direction for a register.

use std::fmt::Display;

use serde::{Deserialize, Serialize};

/// Allowed access direction for a register.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl Display for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Access::ReadOnly => write!(f, "ReadOnly"),
            Access::WriteOnly => write!(f, "WriteOnly"),
            Access::ReadWrite => write!(f, "ReadWrite"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Access;

    #[test]
    /// MB-R-005 — access is exactly one of `ReadOnly`, `WriteOnly`, `ReadWrite`.
    fn ut_access_display() {
        assert_eq!(Access::ReadOnly.to_string(), "ReadOnly");
        assert_eq!(Access::WriteOnly.to_string(), "WriteOnly");
        assert_eq!(Access::ReadWrite.to_string(), "ReadWrite");
    }
}
