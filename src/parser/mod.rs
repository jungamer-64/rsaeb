/// Source-line normalization pipeline.
mod line;
/// Parser source-location conversion helpers.
mod location;
/// Compact rule-line syntax parser.
mod rule_line;

#[cfg(test)]
mod tests;

use crate::error::{ParseError, ParseErrorKind, ParseLimitError};
use crate::limits::SourceByteCount;
use crate::policy::ParsePolicy;
use crate::program::ParsedRuleSink;
use crate::source::{RawProgramSource, SourceLineNumber};

use line::{CompactCodeLineKind, RawSourceLine};
use location::source_line_number;

/// Parses source bytes into a target-specific rule sink.
///
/// # Errors
///
/// Returns `ParseError` if source location conversion, line compaction, rule
/// parsing, or parsed-rule storage fails. Returns the sink's target-shape
/// error after syntax has been fully checked if the parsed source does not
/// match the requested program shape.
pub(crate) fn parse_rules_into<P, S>(source: RawProgramSource<'_>) -> Result<S::Output, S::Error>
where
    P: ParsePolicy,
    S: ParsedRuleSink,
{
    ensure_source_within_limit::<P>(source)?;

    let mut sink = S::new();

    for (zero_based_line, raw_line) in source.as_bytes().split(|&byte| byte == b'\n').enumerate() {
        let line_number = source_line_number(zero_based_line)?;
        let compact_code = RawSourceLine::new(line_number, raw_line, P::CODE_LINE_BYTE_LIMIT)
            .into_code_line()?
            .into_compact_line()?;

        let non_empty_code = match compact_code.classify() {
            CompactCodeLineKind::Blank => continue,
            CompactCodeLineKind::Rule(line) => line,
        };

        let parsed_rule = non_empty_code
            .into_rule_syntax()?
            .parse(P::PAYLOAD_BYTE_LIMIT)?;

        sink.push_parsed_rule(parsed_rule, P::RULE_LIMIT)?;
    }

    sink.finish()
}

/// Checks raw source length before line parsing starts.
///
/// # Errors
///
/// Returns `ParseError` if the source length exceeds parser limits.
fn ensure_source_within_limit<P: ParsePolicy>(
    source: RawProgramSource<'_>,
) -> Result<(), ParseError> {
    let attempted_len = SourceByteCount::new(source.as_bytes().len());
    let limit = P::SOURCE_BYTE_LIMIT;
    if limit.admit(attempted_len).is_some() {
        Ok(())
    } else {
        Err(ParseError::at_line(
            SourceLineNumber::ONE,
            ParseErrorKind::Limit(ParseLimitError::source(limit, attempted_len)),
        ))
    }
}
