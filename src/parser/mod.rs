/// Source-line normalization pipeline.
mod line;
/// Parser source-location conversion helpers.
mod location;
/// Compact rule-line syntax parser.
mod rule_line;

#[cfg(test)]
mod tests;

use crate::error::{ParseError, ParseErrorKind, ParseLimitError};
use crate::limits::{ParseLimits, SourceByteCount};
use crate::program::{RuleSet, RuleSetBuilder};
use crate::source::{ProgramSource, SourceLineNumber};

use line::RawSourceLine;
use location::source_line_number;

/// Parses source bytes into a typed program.
///
/// # Errors
///
/// Returns `ParseError` if source location conversion, line compaction, rule
/// parsing, or parsed-rule storage fails.
pub(crate) fn parse_rules_impl(
    source: ProgramSource<'_>,
    limits: ParseLimits,
) -> Result<RuleSet, ParseError> {
    ensure_source_within_limit(source, limits)?;

    let mut rule_set = RuleSetBuilder::new();

    for (zero_based_line, raw_line) in source.as_bytes().split(|&byte| byte == b'\n').enumerate() {
        let line_number = source_line_number(zero_based_line)?;
        let compact_code = RawSourceLine::new(line_number, raw_line, limits.code_line_byte_limit())
            .into_code_line()?
            .into_compact_line()?;

        let Some(non_empty_code) = compact_code.into_non_empty() else {
            continue;
        };

        let parsed_rule = non_empty_code
            .into_rule_syntax()?
            .parse(limits.payload_byte_limit())?;

        rule_set.push_parsed_rule(parsed_rule, limits.rule_limit())?;
    }

    Ok(rule_set.finish())
}

/// Checks raw source length before line parsing starts.
///
/// # Errors
///
/// Returns `ParseError` if the source length exceeds parser limits.
fn ensure_source_within_limit(
    source: ProgramSource<'_>,
    limits: ParseLimits,
) -> Result<(), ParseError> {
    let attempted_len = SourceByteCount::new(source.as_bytes().len());
    let limit = limits.source_byte_limit();
    if limit.accepts(attempted_len) {
        return Ok(());
    }

    Err(ParseError::at_line(
        SourceLineNumber::ONE,
        ParseErrorKind::Limit(ParseLimitError::source(limit, attempted_len)),
    ))
}
