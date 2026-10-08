//! The `playbackMetadata` namespace: what a group is playing.

use serde::Deserialize;
use url::Url;

use crate::{Error, GroupHandle, protocol::NoParams};

const NAMESPACE: &str = "playbackMetadata";

impl GroupHandle {
    pub async fn get_metadata_status(&self) -> Result<MetadataStatus, Error> {
        self.conn
            .request(NAMESPACE, "getMetadataStatus", self.target(), &NoParams {})
            .await
    }
}

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
    /// Absent when the group is idle.
    pub name: Option<String>,
    pub r#type: Option<String>,
    pub service: Option<Service>,
    pub image_url: Option<ImageUrl>,
    #[serde(default)]
    pub tags: Vec<Tag>,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct UniversalMusicObjectId {
    pub service_id: Option<String>,
    pub object_id: String,
    pub account_id: Option<String>,
}

/// The URL of an image (album art...).
///
/// Usually absolute, but images served by the player itself are given as a path relative to the
/// player's HTTP server, e.g. `/getaa?s=1&u=x-sonos-spotify...`. Use [`ImageUrl::resolve`] to get
/// a URL that can be fetched.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct ImageUrl(String);

impl ImageUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The absolute URL of the image, resolving relative URLs against the player that sent it
    /// (identified by its websocket URL, e.g. [`Connection::websocket_url`](crate::Connection)).
    pub fn resolve(&self, player_websocket_url: &Url) -> Option<Url> {
        match Url::parse(&self.0) {
            Ok(url) => Some(url),
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                let mut base = Url::parse("http://localhost:1400/").expect("valid static URL");
                base.set_host(player_websocket_url.host_str()).ok()?;
                base.join(&self.0).ok()
            }
            Err(_) => None,
        }
    }
}

fn true_bool() -> bool {
    true
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackType {
    #[default]
    Track,
    /// A track type this crate doesn't know about yet.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Service {
    pub name: String,
    pub id: Option<String>,
    pub image_url: Option<ImageUrl>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Track {
    pub id: Option<UniversalMusicObjectId>,
    #[serde(default)]
    pub r#type: TrackType,
    pub name: Option<String>,
    pub image_url: Option<ImageUrl>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Tag {
    TagExplicit,
    /// A tag this crate doesn't know about yet.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Album {
    pub id: Option<UniversalMusicObjectId>,
    pub name: String,
    pub artist: Option<Artist>,
    pub image_url: Option<ImageUrl>,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Artist {
    pub id: Option<UniversalMusicObjectId>,
    pub name: String,
    pub image_url: Option<ImageUrl>,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_image_urls() {
        let player = Url::parse("wss://10.10.237.28:1443/websocket/api").unwrap();
        let relative: ImageUrl =
            serde_json::from_str(r#""/getaa?s=1&u=x-sonos-spotify%3aspotify""#).unwrap();
        assert_eq!(
            relative.resolve(&player).unwrap().as_str(),
            "http://10.10.237.28:1400/getaa?s=1&u=x-sonos-spotify%3aspotify"
        );
        let absolute: ImageUrl = serde_json::from_str(r#""https://example.com/a.jpg""#).unwrap();
        assert_eq!(
            absolute.resolve(&player).unwrap().as_str(),
            "https://example.com/a.jpg"
        );
    }

    #[test]
    fn decodes_tracks_with_relative_image_urls() {
        // From a Sonos playlist of Spotify tracks.
        let track: Track = serde_json::from_str(
            r#"{"_objectType":"track","type":"track","name":"Song","imageUrl":"/getaa?s=1&u=x-sonos-spotify%3aspotify%253atrack%253a3yWuTOYDztXjZxdE2cIRUa%3fsid%3d12%26flags%3d8232%26sn%3d77"}"#,
        )
        .unwrap();
        assert!(track.image_url.unwrap().as_str().starts_with("/getaa?"));
    }

    #[test]
    fn decodes_idle_metadata_status() {
        // Sent by an idle Move and Roam 2 (firmware 97.1).
        let status: MetadataStatus = serde_json::from_str(
            r#"{"_objectType":"metadataStatus","container":{"_objectType":"container","images":[]}}"#,
        )
        .unwrap();
        let container = status.container.unwrap();
        assert_eq!(container.name, None);
        assert!(status.current_item.is_none());
    }
}
