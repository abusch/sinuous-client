//! The `groups` namespace: household topology and group membership.

use iddqd::{IdHashItem, id_upcast};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    Connection, Error, GroupHandle, GroupId, PlayerId, playback::PlaybackState, protocol::NoParams,
};

const NAMESPACE: &str = "groups";

impl Connection {
    /// Get the groups and players in the household.
    pub async fn get_groups(&self) -> Result<Groups, Error> {
        self.request(
            NAMESPACE,
            "getGroups",
            self.household_target(),
            &NoParams {},
        )
        .await
    }

    /// Create a new group from the given players.
    pub async fn create_group(&self, player_ids: &[PlayerId]) -> Result<Group, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            player_ids: &'a [PlayerId],
        }

        let info: GroupInfo = self
            .request(
                NAMESPACE,
                "createGroup",
                self.household_target(),
                &Params { player_ids },
            )
            .await?;
        Ok(info.group)
    }
}

impl GroupHandle {
    /// Add and/or remove players from this group.
    pub async fn modify_members(
        &self,
        player_ids_to_add: &[PlayerId],
        player_ids_to_remove: &[PlayerId],
    ) -> Result<Group, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            player_ids_to_add: &'a [PlayerId],
            player_ids_to_remove: &'a [PlayerId],
        }

        let info: GroupInfo = self
            .conn
            .request(
                NAMESPACE,
                "modifyGroupMembers",
                self.target(),
                &Params {
                    player_ids_to_add,
                    player_ids_to_remove,
                },
            )
            .await?;
        Ok(info.group)
    }

    /// Replace the players in this group.
    pub async fn set_members(&self, player_ids: &[PlayerId]) -> Result<Group, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            player_ids: &'a [PlayerId],
        }

        let info: GroupInfo = self
            .conn
            .request(
                NAMESPACE,
                "setGroupMembers",
                self.target(),
                &Params { player_ids },
            )
            .await?;
        Ok(info.group)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Groups {
    pub groups: Vec<Group>,
    pub players: Vec<Player>,
    /// Set when the player's view of the household is incomplete, e.g. while players are still
    /// being discovered.
    #[serde(default)]
    pub partial: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
struct GroupInfo {
    group: Group,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub coordinator_id: PlayerId,
    pub playback_state: Option<PlaybackState>,
    pub player_ids: Vec<PlayerId>,
}

impl IdHashItem for Group {
    type Key<'a> = &'a GroupId;

    fn key(&self) -> Self::Key<'_> {
        &self.id
    }

    id_upcast!();
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    /// Connect here to control groups this player coordinates.
    pub websocket_url: Url,
    pub api_version: String,
    pub min_api_version: String,
    pub software_version: String,
    pub capabilities: Capabilities,
    pub is_unregistered: bool,
    pub device_ids: Vec<PlayerId>,
    pub devices: Vec<DeviceInfo>,
    // TODO: zone info?
}

impl IdHashItem for Player {
    type Key<'a> = &'a PlayerId;

    fn key(&self) -> Self::Key<'_> {
        &self.id
    }

    id_upcast!();
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: PlayerId,
    pub name: String,
    pub serial_number: String,
    pub model: String,
    pub model_display_name: String,
    pub color: Option<String>,
    pub capabilities: Capabilities,
    pub api_version: String,
    pub min_api_version: String,
    pub websocket_url: Url,
    pub software_version: String,
    pub hw_version: String,
    pub sw_gen: u64,
    // TODO: versions, quarantine_reasons
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Capabilities(Vec<Capability>);

impl Capabilities {
    pub fn contains(&self, capability: Capability) -> bool {
        self.0.contains(&capability)
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Capability {
    /// The player can produce audio. You can target it for playback.
    Playback,
    /// The player can send commands and receive events over the internet.
    Cloud,
    /// The player is a home theater source. It can reproduce the audio from a home theater system, typically delivered by S/PDIF or HDMI.
    HtPlayback,
    /// The player can control the home theater power state. For example, it can switch a connected TV on or off.
    HtPowerState,
    /// Added in version 1.5.1. The player can host AirPlay streams. This capability is present when the device is advertising AirPlay support.
    Airplay,
    /// Added in version 1.6.0. The player has an analog line-in. See Using Line-In on Sonos on the Sonos Support site for more details about the line-in capabilities of our players.
    LineIn,
    /// Added in version 1.7.0. The device is capable of playing audio clip notifications. See the audioClip namespace for details.
    AudioClip,
    /// Added in version 1.10.0. The device supports the voice namespace (not yet implemented).
    Voice,
    /// Added in version 1.10.0. The component device is capable of detecting connected speaker drivers.
    SpeakerDetection,
    /// Added in version 1.11.1. The device supports fixed volume. See setPlayerSettings and the groups object for details.
    FixedVolume,
    /// The device has a physical microphone switch
    MicrophoneSwitch,
    /// The device can be controlled via Infra Red
    IrControl,
    Hdmi,
    Svc,
    /// A capability this crate doesn't know about yet.
    #[serde(other)]
    Unknown,
}

/// Sent instead of a reply when a group-scoped command targets a group the player doesn't
/// coordinate, and as an event when a subscribed group moves or disappears.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct GroupCoordinatorChanged {
    pub group_status: GroupStatus,
    pub group_name: Option<String>,
    /// The websocket of the group's new coordinator (when the group has moved).
    pub websocket_url: Option<Url>,
    /// The group's new coordinator (when the group has moved).
    pub player_id: Option<PlayerId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum GroupStatus {
    /// The group no longer exists.
    #[serde(rename = "GROUP_STATUS_GONE")]
    Gone,
    /// The group is coordinated by another player.
    #[serde(rename = "GROUP_STATUS_MOVED")]
    Moved,
    /// The group's ID changed.
    #[serde(rename = "GROUP_STATUS_UPDATED")]
    Updated,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_capabilities_do_not_fail_decoding() {
        let caps: Capabilities =
            serde_json::from_str(r#"["PLAYBACK", "SOMETHING_NEW", "LINE_IN"]"#).unwrap();
        assert!(caps.contains(Capability::Playback));
        assert!(caps.contains(Capability::LineIn));
        assert!(caps.contains(Capability::Unknown));
        assert!(!caps.contains(Capability::Airplay));
    }
}
