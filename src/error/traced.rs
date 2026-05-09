use core::error::Error;

use super::RunError;

/// Error returned by fallible tracing APIs.
#[derive(Debug, PartialEq, Eq)]
pub enum TracedRunError<E> {
    /// Parser/runtime execution failed.
    Run(RunError),
    /// The user-provided trace sink failed.
    Trace(E),
}

impl<E> Error for TracedRunError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Run(error) => Some(error),
            Self::Trace(error) => Some(error),
        }
    }
}

impl<E> From<RunError> for TracedRunError<E> {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}
