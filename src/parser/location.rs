use crate::allocation::{AllocationContext, AllocationError};
use crate::error::{ParseError, ParseErrorKind};
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
        parse_allocation_error(
            SourceLineNumber::MAX,
            AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
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
        parse_allocation_error(
            line_number,
            AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
        )
    })
}
