//! The `alarms` namespace: the household's alarms.
//!
//! Only reading them is supported for now.

use std::{fmt, time::Duration};

use serde::{Deserialize, Deserializer, de::Error as _};

use crate::{
    AlarmId, Connection, Error, PlayerId, history::ContentType, iso_duration,
    playback_metadata::UniversalMusicObjectId, protocol::NoParams,
};

const NAMESPACE: &str = "alarms";

impl Connection {
    /// Get the household's alarms.
    pub async fn get_alarms(&self) -> Result<AlarmList, Error> {
        self.household_request(NAMESPACE, "getAlarms", &NoParams {})
            .await
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct AlarmList {
    /// Changes whenever an alarm does; see the `versionChanged` event.
    pub version: Option<String>,
    pub alarms: Vec<Alarm>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Alarm {
    pub alarm_id: AlarmId,
    pub enabled: bool,
    pub state: AlarmState,
    pub description: AlarmDescription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AlarmState {
    /// Enabled, waiting for its next start time.
    #[serde(rename = "ALARM_PENDING")]
    Pending,
    #[serde(rename = "ALARM_DISABLED")]
    Disabled,
    /// A state this crate doesn't know about yet, e.g. while the alarm rings.
    #[serde(other)]
    Unknown,
}

/// What an alarm plays, when and where.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct AlarmDescription {
    pub recurrence: Recurrence,
    /// When the alarm starts, in the household's local time.
    pub start_time: TimeOfDay,
    /// How long the alarm plays for. Zero when no duration is set.
    #[serde(deserialize_with = "iso_duration::deserialize")]
    pub total_duration: Duration,
    pub content: Option<AlarmContent>,
    pub actuator: Actuator,
    /// e.g. `NORMAL` or `SHUFFLE`.
    pub play_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Recurrence {
    pub frequency: Frequency,
    /// The days a [weekly](Frequency::Weekly) alarm starts on. Empty otherwise.
    #[serde(default)]
    pub days: Vec<Weekday>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Frequency {
    Once,
    /// On some days of the week: see [`Recurrence::days`].
    Weekly,
    /// A frequency this crate doesn't know about yet.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub enum Weekday {
    #[serde(rename = "MO")]
    Monday,
    #[serde(rename = "TU")]
    Tuesday,
    #[serde(rename = "WE")]
    Wednesday,
    #[serde(rename = "TH")]
    Thursday,
    #[serde(rename = "FR")]
    Friday,
    #[serde(rename = "SA")]
    Saturday,
    #[serde(rename = "SU")]
    Sunday,
}

/// A time of day, in the household's local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl fmt::Display for TimeOfDay {
    /// e.g. `07:00`, or `07:00:30` when there are seconds.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)?;
        if self.second != 0 {
            write!(f, ":{:02}", self.second)?;
        }
        Ok(())
    }
}

impl TimeOfDay {
    /// Parse a time sent by players. They send it as a UTC timestamp on 1 January 1970, e.g.
    /// `1970-01-01T07:00:00Z`, but it's really a local time: the UPnP interface gives the same
    /// alarm's start time as `07:00:00` in the household's time zone.
    fn parse(text: &str) -> Option<Self> {
        let (_, time) = text.split_once('T')?;
        let time = time.strip_suffix('Z').unwrap_or(time);
        let mut parts = time.splitn(3, ':');
        let hour = parts.next()?.parse().ok().filter(|hour| *hour < 24)?;
        let minute = parts.next()?.parse().ok().filter(|minute| *minute < 60)?;
        // Ignore fractions of a second.
        let second = parts.next()?.split('.').next()?;
        let second = second.parse().ok().filter(|second| *second < 60)?;
        Some(Self {
            hour,
            minute,
            second,
        })
    }
}

impl<'de> Deserialize<'de> for TimeOfDay {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| D::Error::custom(format!("invalid time {text:?}")))
    }
}

/// What an alarm plays.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct AlarmContent {
    /// [`Playlist`](ContentType::Playlist) also covers Sonos playlists, and the Sonos chime is
    /// [`Other`](ContentType::Other).
    pub r#type: ContentType,
    pub id: UniversalMusicObjectId,
}

impl AlarmContent {
    /// Whether the alarm plays the Sonos chime rather than music.
    pub fn is_chime(&self) -> bool {
        self.id.service_id.as_deref() == Some("buzzer")
    }
}

/// Where an alarm plays, and how loud.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Actuator {
    pub target: ActuatorTarget,
    /// What the alarm plays on: a player id, for the known [targets](ActuatorTarget).
    pub id: String,
    /// 0 to 100.
    pub volume: u8,
}

impl Actuator {
    /// The player the alarm plays on.
    pub fn player_id(&self) -> Option<PlayerId> {
        match self.target {
            ActuatorTarget::Player | ActuatorTarget::PlayerWithinGroup => {
                Some(PlayerId::new(self.id.as_str()))
            }
            ActuatorTarget::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActuatorTarget {
    /// Only the player.
    Player,
    /// The player and the rooms grouped with it when the alarm starts ("include grouped rooms"
    /// in the Sonos app).
    PlayerWithinGroup,
    /// A target this crate doesn't know about yet.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_alarms() {
        // Trimmed down from a player's response.
        let list: AlarmList = serde_json::from_str(
            r#"{"_objectType":"alarmList","alarms":[{"_objectType":"alarm","description":{"_objectType":"alarmDescription","recurrence":{"_objectType":"recurrenceRule","frequency":"ONCE","days":[]},"startTime":"1970-01-01T07:00:00Z","totalDuration":"PT0S","content":{"_objectType":"content","type":"playlist","id":{"_objectType":"universalMusicObjectId","serviceId":"12","objectId":"spotify:playlist:0QRTdg9f8fuAIvh7WTq21j","accountId":"sn_77"},"metadata":{"_objectType":"contentMetadataBlob"}},"actuator":{"_objectType":"actuator","target":"PLAYER","id":"RINCON_C43875CF926E01400","volume":25},"playMode":"SHUFFLE"},"alarmId":"1","enabled":false,"state":"ALARM_DISABLED"},{"_objectType":"alarm","description":{"_objectType":"alarmDescription","recurrence":{"_objectType":"recurrenceRule","frequency":"WEEKLY","days":["MO","TU","WE","TH","FR"]},"startTime":"1970-01-01T07:00:00Z","totalDuration":"PT2H0M0S","content":{"_objectType":"content","type":"other","id":{"_objectType":"universalMusicObjectId","serviceId":"buzzer","objectId":"0"},"metadata":{"_objectType":"contentMetadataBlob"}},"actuator":{"_objectType":"actuator","target":"PLAYER_WITHIN_GROUP","id":"RINCON_B8E937B0A17401400","volume":20},"playMode":"SHUFFLE"},"alarmId":"22","enabled":true,"state":"ALARM_PENDING"},{"_objectType":"alarm","description":{"_objectType":"alarmDescription","recurrence":{"_objectType":"recurrenceRule","frequency":"ONCE","days":[]},"startTime":"1970-01-01T08:00:00Z","totalDuration":"PT0S","content":{"_objectType":"content","type":"playlist","id":{"_objectType":"universalMusicObjectId","serviceId":"65435","objectId":"0","accountId":"sn_112887175"},"metadata":{"_objectType":"contentMetadataBlob"}},"actuator":{"_objectType":"actuator","target":"PLAYER","id":"RINCON_B8E937B0A17401400","volume":20},"playMode":"NORMAL"},"alarmId":"25","enabled":true,"state":"ALARM_PENDING"}],"version":"RINCON_B8E937B0A17401400:27"}"#,
        )
        .unwrap();
        assert_eq!(list.version.as_deref(), Some("RINCON_B8E937B0A17401400:27"));
        let [once, weekdays, sonos_playlist] = &list.alarms[..] else {
            panic!("expected three alarms, got {:?}", list.alarms);
        };

        assert_eq!(once.alarm_id, AlarmId::new("1"));
        assert!(!once.enabled);
        assert_eq!(once.state, AlarmState::Disabled);
        let description = &once.description;
        assert_eq!(description.recurrence.frequency, Frequency::Once);
        assert!(description.recurrence.days.is_empty());
        assert_eq!(description.start_time.to_string(), "07:00");
        assert_eq!(description.total_duration, Duration::ZERO);
        assert!(!description.content.as_ref().unwrap().is_chime());
        assert_eq!(
            description.actuator.player_id(),
            Some(PlayerId::new("RINCON_C43875CF926E01400"))
        );
        assert_eq!(description.actuator.volume, 25);
        assert_eq!(description.play_mode.as_deref(), Some("SHUFFLE"));

        assert_eq!(weekdays.state, AlarmState::Pending);
        let description = &weekdays.description;
        assert_eq!(description.recurrence.frequency, Frequency::Weekly);
        assert_eq!(
            description.recurrence.days,
            [
                Weekday::Monday,
                Weekday::Tuesday,
                Weekday::Wednesday,
                Weekday::Thursday,
                Weekday::Friday
            ]
        );
        assert_eq!(description.total_duration, Duration::from_secs(2 * 3600));
        let content = description.content.as_ref().unwrap();
        assert!(content.is_chime());
        assert_eq!(content.r#type, ContentType::Other);
        assert_eq!(
            description.actuator.target,
            ActuatorTarget::PlayerWithinGroup
        );
        assert_eq!(
            description.actuator.player_id(),
            Some(PlayerId::new("RINCON_B8E937B0A17401400"))
        );

        let description = &sonos_playlist.description;
        assert_eq!(description.start_time.to_string(), "08:00");
        assert_eq!(description.play_mode.as_deref(), Some("NORMAL"));
        assert_eq!(description.actuator.target, ActuatorTarget::Player);
        let content = description.content.as_ref().unwrap();
        assert_eq!(content.r#type, ContentType::Playlist);
        assert_eq!(content.id.object_id, "0");
    }

    #[test]
    fn decodes_radio_content() {
        // A Sonos Radio station.
        let content: AlarmContent = serde_json::from_value(serde_json::json!({
            "_objectType": "content",
            "type": "program",
            "id": {"_objectType": "universalMusicObjectId", "serviceId": "303", "objectId": "sonos:154252", "accountId": "sn_1"},
            "metadata": {"_objectType": "contentMetadataBlob"}
        }))
        .unwrap();
        assert_eq!(content.r#type, ContentType::Program);
        assert_eq!(content.id.object_id, "sonos:154252");
        assert!(!content.is_chime());
    }

    #[test]
    fn parses_times_of_day() {
        let time = |text| TimeOfDay::parse(text);
        assert_eq!(
            time("1970-01-01T07:05:30Z"),
            Some(TimeOfDay {
                hour: 7,
                minute: 5,
                second: 30
            })
        );
        assert_eq!(
            time("1970-01-01T23:59:00.250Z").unwrap().to_string(),
            "23:59"
        );
        assert_eq!(
            time("1970-01-01T07:05:30Z").unwrap().to_string(),
            "07:05:30"
        );
        for text in [
            "07:00:00",
            "1970-01-01T24:00:00Z",
            "1970-01-01T07:60:00Z",
            "1970-01-01T07:00Z",
        ] {
            assert_eq!(time(text), None, "{text:?}");
        }
    }

    #[test]
    fn keeps_unknown_states() {
        let state: AlarmState = serde_json::from_str(r#""ALARM_RINGING""#).unwrap();
        assert_eq!(state, AlarmState::Unknown);
        let frequency: Frequency = serde_json::from_str(r#""MONTHLY""#).unwrap();
        assert_eq!(frequency, Frequency::Unknown);
        let actuator: Actuator = serde_json::from_value(serde_json::json!({
            "_objectType": "actuator", "target": "ZONE", "id": "x", "volume": 10
        }))
        .unwrap();
        assert_eq!(actuator.target, ActuatorTarget::Unknown);
        assert_eq!(actuator.player_id(), None);
    }
}
