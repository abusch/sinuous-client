//! The `groups` namespace: household topology and group membership.

use iddqd::{IdHashItem, id_upcast};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    Connection, Error, GroupHandle, GroupId, PlayerId,
    playback::PlaybackState,
    protocol::{NoParams, skip_invalid},
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

    /// Create a new group from the given players. The first player becomes its coordinator.
    ///
    /// When sent to a player that isn't part of the new group, the reply comes before the group
    /// is actually created, so commands sent right after may be applied first. Send it to the
    /// first player instead, or use [`Household::create_group`](crate::Household::create_group).
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
    /// Groups that can't be decoded are left out, with a warning.
    #[serde(deserialize_with = "skip_invalid")]
    pub groups: Vec<Group>,
    /// Players that can't be decoded are left out, with a warning.
    #[serde(deserialize_with = "skip_invalid")]
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
    #[serde(default)]
    pub name: String,
    /// Connect here to control groups this player coordinates.
    pub websocket_url: Url,
    /// Empty if the player didn't say, like the other strings below.
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub min_api_version: String,
    #[serde(default)]
    pub software_version: String,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub is_unregistered: bool,
    #[serde(default)]
    pub device_ids: Vec<PlayerId>,
    /// Devices that can't be decoded are left out, with a warning.
    #[serde(default, deserialize_with = "skip_invalid")]
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
    /// Empty if the device didn't say, like the other strings below.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub serial_number: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_display_name: String,
    pub color: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub min_api_version: String,
    pub websocket_url: Url,
    #[serde(default)]
    pub software_version: String,
    #[serde(default)]
    pub hw_version: String,
    /// 0 if the device didn't say.
    #[serde(default)]
    pub sw_gen: u64,
    // TODO: versions, quarantine_reasons
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
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
    /// The group changed, e.g. players joined or left it. It keeps its ID, and subscriptions to
    /// it carry on.
    #[serde(rename = "GROUP_STATUS_UPDATED")]
    Updated,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn decodes_players_with_few_details() {
        let groups: Groups = serde_json::from_value(json!({
            "_objectType": "groups",
            "groups": [
                {"_objectType": "group", "id": "A:1", "name": "Kitchen", "coordinatorId": "A",
                 "playerIds": ["A"]}
            ],
            "players": [
                {"_objectType": "player", "id": "A",
                 "websocketUrl": "wss://10.0.0.1:1443/websocket/api",
                 "devices": [{"_objectType": "deviceInfo", "id": "A",
                              "websocketUrl": "wss://10.0.0.1:1443/websocket/api"}]}
            ]
        }))
        .unwrap();
        let player = &groups.players[0];
        assert_eq!(player.id, PlayerId::new("A"));
        assert_eq!(player.software_version, "");
        assert!(!player.is_unregistered);
        assert_eq!(player.capabilities.iter().count(), 0);
        assert_eq!(player.devices[0].sw_gen, 0);
    }

    #[test]
    fn skips_what_cannot_be_decoded() {
        let groups: Groups = serde_json::from_value(json!({
            "_objectType": "groups",
            "groups": [
                {"_objectType": "group", "id": "A:1", "name": "Kitchen", "coordinatorId": "A",
                 "playerIds": ["A"]},
                {"_objectType": "group", "id": "B:1", "name": "Office"}
            ],
            "players": [
                {"_objectType": "player", "id": "A",
                 "websocketUrl": "wss://10.0.0.1:1443/websocket/api",
                 "devices": [{"_objectType": "deviceInfo", "id": "A"}]},
                {"_objectType": "player", "id": "B", "websocketUrl": "not a url"}
            ]
        }))
        .unwrap();
        let group_ids: Vec<_> = groups.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(group_ids, ["A:1"]);
        let player_ids: Vec<_> = groups.players.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(player_ids, ["A"]);
        assert!(groups.players[0].devices.is_empty());
    }

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
