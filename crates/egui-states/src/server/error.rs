use std::fmt;

pub type Result<T> = std::result::Result<T, ServerError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerError {
    message: String,
}

impl ServerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServerError {}

impl From<String> for ServerError {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ServerError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}
