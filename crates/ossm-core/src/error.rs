use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreError {
    StaleVersion,
    InvalidConfig,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleVersion => write!(f, "Stale causal version"),
            Self::InvalidConfig => write!(f, "Invalid config"),
        }
    }
}

pub type Result<T> = core::result::Result<T, CoreError>;
