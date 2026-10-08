//! Events pushed by players for subscribed namespaces.

use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use tracing::warn;

use crate::{
    Connection, Error, GroupId, HouseholdId, PlayerId,
    groups::{GroupCoordinatorChanged, Groups},
    home_theater::HomeTheaterOptions,
    playback::{PlaybackError, PlaybackStatus},
    playback_metadata::MetadataStatus,
    protocol::{NoParams, Target, object_type},
    volume::Volume,
};

/// An event sent by a player.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Event {
    /// The namespace the event belongs to, e.g. `playback`.
    pub namespace: String,
    /// The name of the event, e.g. `playbackStatus`.
    pub name: String,
    pub household_id: Option<HouseholdId>,
    /// Set for group-scoped events.
    pub group_id: Option<GroupId>,
    /// Set for player-scoped events.
    pub player_id: Option<PlayerId>,
    pub payload: EventPayload,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)] // Events are infrequent; boxing would only make matching clunkier.
pub enum EventPayload {
    Groups(Groups),
    PlaybackStatus(PlaybackStatus),
    PlaybackError(PlaybackError),
    MetadataStatus(MetadataStatus),
    GroupVolume(Volume),
    PlayerVolume(Volume),
    HomeTheaterOptions(HomeTheaterOptions),
    /// The favorites or playlists (see [`Event::namespace`]) have changed.
    VersionChanged(VersionChanged),
    GroupCoordinatorChanged(GroupCoordinatorChanged),
    /// An event this crate doesn't know how to decode (yet). Contains the raw body.
    Unknown(Value),
}

impl EventPayload {
    pub(crate) fn from_body(body: Value) -> Self {
        let Some(ty) = object_type(&body) else {
            return Self::Unknown(body);
        };
        match ty {
            "groups" => decode(body, Self::Groups),
            "playbackStatus" => decode(body, Self::PlaybackStatus),
            "playbackError" => decode(body, Self::PlaybackError),
            "metadataStatus" => decode(body, Self::MetadataStatus),
            "groupVolume" => decode(body, Self::GroupVolume),
            "playerVolume" => decode(body, Self::PlayerVolume),
            "homeTheaterOptions" => decode(body, Self::HomeTheaterOptions),
            "versionChanged" => decode(body, Self::VersionChanged),
            "groupCoordinatorChanged" => decode(body, Self::GroupCoordinatorChanged),
            _ => Self::Unknown(body),
        }
    }
}

/// Decode a known event body, falling back to [`EventPayload::Unknown`] so that a model that
/// doesn't match what the player sends doesn't cause events to be lost.
fn decode<T: DeserializeOwned>(body: Value, wrap: fn(T) -> EventPayload) -> EventPayload {
    match T::deserialize(&body) {
        Ok(payload) => wrap(payload),
        Err(e) => {
            warn!("Failed to decode {:?} event: {e}", object_type(&body));
            EventPayload::Unknown(body)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct VersionChanged {
    pub version: String,
}

/// A namespace to receive events for.
///
/// Players send an initial event with the current state right after subscribing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Subscription {
    Groups,
    Favorites,
    Playlists,
    Playback(GroupId),
    PlaybackMetadata(GroupId),
    GroupVolume(GroupId),
    PlayerVolume(PlayerId),
    HomeTheater(PlayerId),
}

impl Subscription {
    fn namespace(&self) -> &'static str {
        match self {
            Self::Groups => "groups",
            Self::Favorites => "favorites",
            Self::Playlists => "playlists",
            Self::Playback(_) => "playback",
            Self::PlaybackMetadata(_) => "playbackMetadata",
            Self::GroupVolume(_) => "groupVolume",
            Self::PlayerVolume(_) => "playerVolume",
            Self::HomeTheater(_) => "homeTheater",
        }
    }

    /// The group of a group-scoped subscription.
    pub(crate) fn group_id(&self) -> Option<&GroupId> {
        match self {
            Self::Playback(id) | Self::PlaybackMetadata(id) | Self::GroupVolume(id) => Some(id),
            _ => None,
        }
    }

    fn target<'a>(&'a self, conn: &'a Connection) -> Target<'a> {
        match self {
            Self::Groups | Self::Favorites | Self::Playlists => conn.household_target(),
            Self::Playback(id) | Self::PlaybackMetadata(id) | Self::GroupVolume(id) => {
                Target::Group(id)
            }
            Self::PlayerVolume(id) | Self::HomeTheater(id) => Target::Player(id),
        }
    }
}

impl Connection {
    /// Start receiving events for a namespace. Events are delivered to [`Connection::events`].
    pub async fn subscribe(&self, subscription: &Subscription) -> Result<(), Error> {
        self.command(
            subscription.namespace(),
            "subscribe",
            subscription.target(self),
            &NoParams {},
        )
        .await
    }

    pub async fn unsubscribe(&self, subscription: &Subscription) -> Result<(), Error> {
        self.command(
            subscription.namespace(),
            "unsubscribe",
            subscription.target(self),
            &NoParams {},
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn unknown_object_types_are_kept() {
        let body = json!({"_objectType": "tvAudioSignalStatus", "signalDetected": false});
        let EventPayload::Unknown(raw) = EventPayload::from_body(body.clone()) else {
            panic!("expected unknown payload");
        };
        assert_eq!(raw, body);
    }

    #[test]
    fn undecodable_known_types_are_kept() {
        let body = json!({"_objectType": "groupVolume", "volume": "loud"});
        assert!(matches!(
            EventPayload::from_body(body),
            EventPayload::Unknown(_)
        ));
    }

    #[test]
    fn decodes_version_changed() {
        let body = json!({"_objectType": "versionChanged", "version": "RINCON_5:19"});
        let EventPayload::VersionChanged(changed) = EventPayload::from_body(body) else {
            panic!("expected version changed");
        };
        assert_eq!(changed.version, "RINCON_5:19");
    }
}
