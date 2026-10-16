//! ISO 8601 durations made of hours, minutes and seconds, e.g. `PT1H5M7S`, as used by players.

use std::time::Duration;

use serde::{Deserialize, Deserializer, de::Error as _};

/// Format a duration the way players expect it, e.g. `PT1H5M7S`, rounded down to whole seconds.
pub(crate) fn format(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("PT{}H{}M{}S", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Parse a duration whose hours, minutes and seconds may each be left out, e.g. `PT1H5M7S` or
/// `PT45M`.
pub(crate) fn parse(text: &str) -> Option<Duration> {
    let mut rest = text.strip_prefix("PT").filter(|rest| !rest.is_empty())?;
    let mut total = Duration::ZERO;
    for (unit, secs) in [('H', 3600), ('M', 60)] {
        if let Some((count, tail)) = rest.split_once(unit) {
            let component = Duration::from_secs(count.parse::<u64>().ok()?.checked_mul(secs)?);
            total = total.checked_add(component)?;
            rest = tail;
        }
    }
    if let Some(count) = rest.strip_suffix('S') {
        let component = Duration::try_from_secs_f64(count.parse().ok()?).ok()?;
        total = total.checked_add(component)?;
    } else if !rest.is_empty() {
        return None;
    }
    Some(total)
}

/// For `#[serde(deserialize_with)]`.
pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Duration, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse(&text).ok_or_else(|| D::Error::custom(format!("invalid duration {text:?}")))
}

/// For `#[serde(deserialize_with)]`, on an `Option<Duration>` (which also needs `#[serde(default)]`).
pub(crate) fn deserialize_option<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error> {
    let Some(text) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    parse(&text)
        .map(Some)
        .ok_or_else(|| D::Error::custom(format!("invalid duration {text:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(format(Duration::from_secs(3600)), "PT1H0M0S");
        assert_eq!(format(Duration::from_secs(3907)), "PT1H5M7S");
        assert_eq!(format(Duration::from_millis(45_999)), "PT0H0M45S");
        assert_eq!(format(Duration::from_secs(86_399)), "PT23H59M59S");
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse("PT1H5M7S"), Some(Duration::from_secs(3907)));
        assert_eq!(parse("PT45M"), Some(Duration::from_secs(2700)));
        assert_eq!(parse("PT2H"), Some(Duration::from_secs(7200)));
        assert_eq!(parse("PT1.5S"), Some(Duration::from_millis(1500)));
        assert_eq!(parse("PT0H0M0S"), Some(Duration::ZERO));
    }

    #[test]
    fn rejects_malformed_durations() {
        for text in [
            "", "PT", "P1D", "1H", "PTH", "PT1X", "PT1S2M", "PT-1S", "PTinfS",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }
}
