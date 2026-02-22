use iddqd::{IdHashItem, id_upcast};
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub enum SonosObject {
    Groups(Groups),
    Group(Group),
    Player(Player),
    MetadataStatus(MetadataStatus),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Groups {
    pub groups: Vec<Group>,
    pub players: Vec<Player>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct PlayerId(String);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PlaybackState {
    #[serde(rename = "PLAYBACK_STATE_BUFFERING")]
    Buffering,
    #[serde(rename = "PLAYBACK_STATE_IDLE")]
    Idle,
    #[serde(rename = "PLAYBACK_STATE_PAUSED")]
    Paused,
    #[serde(rename = "PLAYBACK_STATE_PLAYING")]
    Playing,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Capabilities(Vec<Capability>);

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
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefixMessage {
    pub namespace: String,
    pub r#type: String,
    #[serde(flatten)]
    pub payload: PrefixMessagePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged, rename_all = "camelCase")]
pub enum PrefixMessagePayload {
    Reply { response: String, success: bool },
    Event { name: String },
}

#[derive(Debug, Clone)]
pub struct SonosMsg(pub PrefixMessage, pub SonosObject);

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct MetadataStatus {
    pub container: Option<Container>,
    pub current_item: Option<QueueItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Container {
    pub name: String,
    pub r#type: String,
    // pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct QueueItem {
    pub track: Track,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Track {
    pub name: String,
    pub album: Album,
    pub artist: Artist,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Album {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Artist {
    pub name: String,
}
