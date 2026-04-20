use iddqd::{IdHashItem, id_upcast};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub enum SonosObject {
    Groups(Groups),
    Group(Group),
    Player(Player),
    PlaybackStatus(PlaybackStatus),
    MetadataStatus(MetadataStatus),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Groups {
    pub groups: Vec<Group>,
    pub players: Vec<Player>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    pub next_item: Option<QueueItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Container {
    pub id: Option<UniversalMusicObjectId>,
    pub name: String,
    pub r#type: Option<String>,
    pub service: Option<Service>,
    pub image_url: Option<Url>,
    #[serde(default)]
    pub tags: Vec<Tags>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct QueueItem {
    pub id: Option<String>,
    pub track: Track,
    #[serde(default)]
    pub deleted: bool,
    // TODO: policies
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct UniversalMusicObjectId {
    pub service_id: Option<String>,
    pub object_id: String,
    pub account_id: Option<String>,
}

fn true_bool() -> bool {
    true
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackType {
    #[default]
    Track,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Service {
    pub name: String,
    pub id: Option<String>,
    pub image_url: Option<Url>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Track {
    pub id: Option<UniversalMusicObjectId>,
    #[serde(default)]
    pub r#type: TrackType,
    pub name: Option<String>,
    pub image_url: Option<Url>,
    pub album: Option<Album>,
    pub artist: Option<Artist>,
    #[serde(default = "true_bool")]
    pub can_crossfade: bool,
    #[serde(default = "true_bool")]
    pub can_skip: bool,
    pub duration_millis: Option<u64>,
    pub replay_gain: Option<i64>,
    pub service: Option<Service>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Tags {
    TagExplicit,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Album {
    pub id: Option<UniversalMusicObjectId>,
    pub name: String,
    pub artist: Option<Artist>,
    pub image_url: Option<Url>,
    #[serde(default)]
    pub tags: Vec<Tags>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Artist {
    pub id: Option<UniversalMusicObjectId>,
    pub name: String,
    pub image_url: Option<Url>,
    #[serde(default)]
    pub tags: Vec<Tags>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct PlaybackStatus {
    pub playback_state: PlaybackState,
    #[serde(default)]
    pub is_ducking: bool,
    pub queue_version: Option<String>,
    pub item_id: Option<String>,
    pub position_millis: Option<i32>,
    pub previous_item_id: Option<String>,
    pub previous_position_millis: Option<i32>,
    // TODO: play modes, actions...
}
