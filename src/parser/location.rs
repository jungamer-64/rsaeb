use crate::allocation::AllocationError;
use crate::error::{ParseError, ParseErrorKind, ParseRepresentationError};
use crate::source::{SourceColumn, SourceLineNumber};

/// Converts an internal parser allocation failure into a source-line parse error.
pub(super) fn parse_allocation_error(
    line_number: SourceLineNumber,
    error: AllocationError,
) -> ParseError {
    ParseError::at_line(line_number, ParseErrorKind::Allocation(error))
}

/// Converts a zero-based source line index into the domain line number.
///
/// # Errors
///
/// Returns `ParseError` if the one-based source line cannot be represented.
pub(super) fn source_line_number(zero_based_line: usize) -> Result<SourceLineNumber, ParseError> {
    SourceLineNumber::from_zero_based(zero_based_line).ok_or_else(|| {
        ParseError::at_line(
            SourceLineNumber::MAX,
            ParseErrorKind::Representation(ParseRepresentationError::SourceLineNumber),
        )
    })
}

/// Converts a zero-based source column index into the domain column number.
///
/// # Errors
///
/// Returns `ParseError` if the one-based source column cannot be represented.
pub(super) fn source_column(
    zero_based_column: usize,
    line_number: SourceLineNumber,
) -> Result<SourceColumn, ParseError> {
    SourceColumn::from_zero_based(zero_based_column).ok_or_else(|| {
        ParseError::at_line(
            line_number,
            ParseErrorKind::Representation(ParseRepresentationError::SourceColumn {
                line: line_number,
            }),
        )
    })
}
