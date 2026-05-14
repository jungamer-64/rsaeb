mod line;
mod location;
mod rule_line;

#[cfg(test)]
mod tests;

use crate::error::ParseError;
use crate::program::{Program, RuleSet};
use crate::source::ProgramSource;

use line::RawSourceLine;
use location::{parse_allocation_error, source_line_number};

pub(crate) fn parse_program_impl(source: ProgramSource<'_>) -> Result<Program, ParseError> {
    let mut rule_set = RuleSet::new();

    for (zero_based_line, raw_line) in source.as_bytes().split(|&byte| byte == b'\n').enumerate() {
        let line_number = source_line_number(zero_based_line)?;
        let compact_code = RawSourceLine::new(line_number, raw_line)
            .into_code_line()?
            .into_compact_line()?;

        let Some(non_empty_code) = compact_code.into_non_empty() else {
            continue;
        };

        let parsed_rule = non_empty_code.into_rule_syntax()?.parse()?;

        rule_set
            .push_parsed_rule(parsed_rule)
            .map_err(|error| parse_allocation_error(line_number, error))?;
    }

    Ok(Program::from_rule_set(rule_set))
}
