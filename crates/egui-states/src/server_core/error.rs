//! Errors returned by the server cores to the front-ends.

use std::fmt;

/// What went wrong, for front-ends that report categories, such as the C
/// ABI's status codes. The text is the same whatever the kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorKind {
    /// The state has no entry at the requested key or index.
    NotFound,
    /// Any other failure; front-ends report it as they always have.
    Other,
}

/// An error of a server core: a category and the message every front-end
/// shows unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoreError {
    pub(crate) kind: ErrorKind,
    pub(crate) message: String,
}

impl CoreError {
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::NotFound,
            message: message.into(),
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for CoreError {
    fn from(message: String) -> Self {
        Self {
            kind: ErrorKind::Other,
            message,
        }
    }
}

impl From<&str> for CoreError {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

impl From<CoreError> for String {
    fn from(error: CoreError) -> Self {
        error.message
    }
}

/// Python reports a core error with its message, as a `str` argument.
#[cfg(feature = "python")]
impl<'py> pyo3::IntoPyObject<'py> for CoreError {
    type Target = pyo3::types::PyString;
    type Output = pyo3::Bound<'py, pyo3::types::PyString>;
    type Error = std::convert::Infallible;

    fn into_pyobject(self, py: pyo3::Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pyo3::types::PyString::new(py, &self.message))
    }
}
