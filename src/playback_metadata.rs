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
    pub image_url: Option<Url>,
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
    pub image_url: Option<Url>,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Artist {
    pub id: Option<UniversalMusicObjectId>,
    pub name: String,
    pub image_url: Option<Url>,
    #[serde(default)]
    pub tags: Vec<Tag>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
