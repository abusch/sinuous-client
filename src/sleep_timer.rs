//! The `sleepTimer` namespace: pause a group's playback after a while.
//!
//! When the timer runs out, the group fades its volume out, pauses, then restores its volume.
//!
//! Durations go over the wire as ISO 8601 durations like `PT1H5M7S`, whose components the
//! players require to be in range: up to 23 hours, 59 minutes and 59 seconds.

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{Error, GroupHandle, protocol::NoParams};

const NAMESPACE: &str = "sleepTimer";

impl GroupHandle {
    pub async fn get_sleep_timer(&self) -> Result<SleepTimerStatus, Error> {
        self.conn
            .request(NAMESPACE, "getSleepTimer", self.target(), &NoParams {})
            .await
    }

    /// Pause playback after `duration`, replacing any timer already set.
    ///
    /// `duration` is rounded down to whole seconds. Players reject durations shorter than a
    /// second or longer than 23:59:59 with an [`Error::Api`].
    pub async fn set_sleep_timer(&self, duration: Duration) -> Result<(), Error> {
        #[derive(Serialize)]
        struct Params {
            duration: String,
        }

        self.conn
            .command(
                NAMESPACE,
                "configureSleepTimer",
                self.target(),
                &Params {
                    duration: format_duration(duration),
                },
            )
            .await
    }

    /// Cancel the sleep timer. Does nothing if none is set.
    pub async fn cancel_sleep_timer(&self) -> Result<(), Error> {
        self.conn
            .command(
                NAMESPACE,
                "configureSleepTimer",
                self.target(),
                &NoParams {},
            )
            .await
    }
}

/// The sleep timer of a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct SleepTimerStatus {
    pub active: bool,
    /// The time left before the volume starts fading out, when the timer is active. It stays
    /// active, with nothing left, while the volume fades out.
    #[serde(
        default,
        rename = "remainingTimeDuration",
        deserialize_with = "deserialize_remaining"
    )]
    pub remaining: Option<Duration>,
}

fn deserialize_remaining<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error> {
    let Some(text) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    parse_duration(&text)
        .map(Some)
        .ok_or_else(|| D::Error::custom(format!("invalid duration {text:?}")))
}

/// Format a duration the way players expect it, e.g. `PT1H5M7S`.
fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("PT{}H{}M{}S", secs / 3600, secs / 60 % 60, secs % 60)
}

/// Parse an ISO 8601 duration made of hours, minutes and seconds, any of which may be left out,
/// e.g. `PT1H5M7S` or `PT45M`.
fn parse_duration(text: &str) -> Option<Duration> {
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

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{Arc, Mutex},
    };

    use serde_json::json;

    use super::*;
    use crate::{
        ConnectOptions, Connection, GroupId, HouseholdId,
        test_support::{fake_player, reply, url},
    };

    async fn connect(addr: SocketAddr) -> Connection {
        let options = ConnectOptions::default()
            .request_timeout(Duration::from_millis(500))
            .household_id(HouseholdId::new("Sonos_1"));
        Connection::connect_to(&url(addr), options).await.unwrap()
    }

    #[test]
    fn decodes_active_status() {
        let status: SleepTimerStatus = serde_json::from_value(json!({
            "_objectType": "sleepTimerStatus",
            "remainingTimeDuration": "PT0H59M43S",
            "active": true
        }))
        .unwrap();
        assert!(status.active);
        assert_eq!(status.remaining, Some(Duration::from_secs(59 * 60 + 43)));
    }

    #[test]
    fn decodes_inactive_status() {
        let status: SleepTimerStatus =
            serde_json::from_value(json!({"_objectType": "sleepTimerStatus", "active": false}))
                .unwrap();
        assert!(!status.active);
        assert_eq!(status.remaining, None);
    }

    #[test]
    fn rejects_invalid_remaining_time() {
        let result = serde_json::from_value::<SleepTimerStatus>(json!({
            "_objectType": "sleepTimerStatus",
            "remainingTimeDuration": "01:00:00",
            "active": true
        }));
        assert!(result.is_err());
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_secs(3600)), "PT1H0M0S");
        assert_eq!(format_duration(Duration::from_secs(3907)), "PT1H5M7S");
        assert_eq!(format_duration(Duration::from_millis(45_999)), "PT0H0M45S");
        assert_eq!(format_duration(Duration::from_secs(86_399)), "PT23H59M59S");
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("PT1H5M7S"), Some(Duration::from_secs(3907)));
        assert_eq!(parse_duration("PT45M"), Some(Duration::from_secs(2700)));
        assert_eq!(parse_duration("PT2H"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("PT1.5S"), Some(Duration::from_millis(1500)));
        assert_eq!(parse_duration("PT0H0M0S"), Some(Duration::ZERO));
    }

    #[test]
    fn rejects_malformed_durations() {
        for text in [
            "", "PT", "P1D", "1H", "PTH", "PT1X", "PT1S2M", "PT-1S", "PTinfS",
        ] {
            assert_eq!(parse_duration(text), None, "{text:?}");
        }
    }

    #[tokio::test]
    async fn sets_and_cancels_the_timer() {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let (addr, _) = fake_player({
            let bodies = bodies.clone();
            move |header, body| {
                assert_eq!(header["namespace"], "sleepTimer");
                assert_eq!(header["command"], "configureSleepTimer");
                assert_eq!(header["groupId"], "RINCON_1:42");
                bodies.lock().unwrap().push(body.clone());
                Some(reply(
                    header,
                    true,
                    json!({"_objectType": "sleepTimerStatus", "active": false}),
                ))
            }
        })
        .await;
        let group = connect(addr).await.group(&GroupId::new("RINCON_1:42"));
        group
            .set_sleep_timer(Duration::from_secs(90 * 60))
            .await
            .unwrap();
        group.cancel_sleep_timer().await.unwrap();
        assert_eq!(
            *bodies.lock().unwrap(),
            [json!({"duration": "PT1H30M0S"}), json!({})]
        );
    }
}
