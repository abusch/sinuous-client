//! The `sleepTimer` namespace: pause a group's playback after a while.
//!
//! When the timer runs out, the group fades its volume out, pauses, then restores its volume.
//!
//! Durations go over the wire as ISO 8601 durations like `PT1H5M7S`, whose components the
//! players require to be in range: up to 23 hours, 59 minutes and 59 seconds.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Error, GroupHandle, iso_duration, protocol::NoParams};

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
                    duration: iso_duration::format(duration),
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
        deserialize_with = "iso_duration::deserialize_option"
    )]
    pub remaining: Option<Duration>,
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
