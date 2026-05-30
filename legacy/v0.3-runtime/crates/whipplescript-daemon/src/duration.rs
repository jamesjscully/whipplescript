use std::time::Duration;

use whipplescript_core::{WhippleScriptError, WhippleScriptResult};

pub fn parse_duration(input: &str) -> WhippleScriptResult<Duration> {
    let trimmed = input.trim();
    let units = [("ms", 1_u64), ("s", 1_000), ("m", 60_000), ("h", 3_600_000)];

    for (suffix, multiplier) in units {
        if let Some(number) = trimmed.strip_suffix(suffix) {
            let value = number.trim().parse::<u64>().map_err(|error| {
                WhippleScriptError::invalid_input(format!("invalid duration {input:?}: {error}"))
            })?;
            return Ok(Duration::from_millis(value.saturating_mul(multiplier)));
        }
    }

    Err(WhippleScriptError::invalid_input(format!(
        "invalid duration {input:?}: expected suffix ms, s, m, or h"
    )))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::parse_duration;

    #[test]
    fn parses_simple_durations() {
        assert_eq!(parse_duration("300ms").unwrap(), Duration::from_millis(300));
        assert_eq!(parse_duration("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
    }
}
