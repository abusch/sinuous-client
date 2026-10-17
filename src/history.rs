//! The `history` namespace: what the household played recently.

use serde::{Deserialize, Serialize};

use crate::{
    Connection, Error,
    playback_metadata::{ImageUrl, UniversalMusicObjectId},
    protocol::{NoParams, true_bool},
};

const NAMESPACE: &str = "history";

impl Connection {
    /// Get the albums, playlists and stations played recently in the household, most recent
    /// first.
    ///
    /// Players only keep the last few dozen. Content played from elsewhere, e.g. from the Spotify
    /// app (Spotify Connect), doesn't seem to be recorded.
    pub async fn get_history(&self) -> Result<History, Error> {
        self.household_request(NAMESPACE, "getHistory", &NoParams {})
            .await
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct History {
    pub info: PageInfo,
    #[serde(rename = "resources")]
    pub items: Vec<HistoryItem>,
}

/// Which part of a longer list was returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct PageInfo {
    /// The number of items returned.
    pub count: u32,
    /// The index of the first item returned.
    pub offset: u32,
    /// The number of items in the whole list.
    pub total: u32,
}

/// Something played recently.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct HistoryItem {
    pub r#type: ContentType,
    pub id: UniversalMusicObjectId,
    pub name: String,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default = "true_bool")]
    pub playable: bool,
    #[serde(default)]
    pub images: Vec<Image>,
}

/// What kind of content something is, e.g. for
/// [`GroupHandle::load_content`](crate::GroupHandle::load_content).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Album,
    Playlist,
    /// A radio station or show, e.g. Sonos Radio or a Spotify artist radio.
    Program,
    /// e.g. the Sonos chime of an [alarm](crate::alarms::AlarmContent).
    Other,
    /// A content type this crate doesn't know about yet.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Image {
    pub url: ImageUrl,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_history() {
        // Trimmed down from a player's response.
        let history: History = serde_json::from_str(
            r#"{"_objectType":"contentPagedResources","info":{"_objectType":"contentPageInfo","count":2,"offset":0,"total":2},"resources":[{"_objectType":"contentResource","type":"album","id":{"_objectType":"universalMusicObjectId","serviceId":"12","objectId":"spotify:album:1au1XG2AdakLtj9TPPNoSc","accountId":"sn_77"},"name":"Summit","explicit":false,"playable":true,"metadata":{"_objectType":"contentMetadataBlob","metadataBlob":"eyJuYW1lIjoiU3VtbWl0In0"},"images":[{"_objectType":"image","url":"https://i.scdn.co/image/ab67616d0000b273551d8b7be2f5ffabd720f568"}]},{"_objectType":"contentResource","type":"program","id":{"_objectType":"universalMusicObjectId","serviceId":"303","objectId":"sonos:3011","accountId":"sn_1"},"name":"Island Steady","explicit":false,"playable":true,"metadata":{"_objectType":"contentMetadataBlob","metadataBlob":"eyJuYW1lIjoiSXNsYW5kIFN0ZWFkeSJ9"},"images":[]}]}"#,
        )
        .unwrap();
        assert_eq!(
            history.info,
            PageInfo {
                count: 2,
                offset: 0,
                total: 2
            }
        );
        let [album, station] = &history.items[..] else {
            panic!("expected two items, got {:?}", history.items);
        };
        assert_eq!(album.r#type, ContentType::Album);
        assert_eq!(album.id.object_id, "spotify:album:1au1XG2AdakLtj9TPPNoSc");
        assert_eq!(album.name, "Summit");
        assert_eq!(
            album.images[0].url.as_str(),
            "https://i.scdn.co/image/ab67616d0000b273551d8b7be2f5ffabd720f568"
        );
        assert_eq!(station.r#type, ContentType::Program);
        assert!(station.playable);
        assert!(station.images.is_empty());
    }

    #[test]
    fn keeps_unknown_content_types() {
        let item: HistoryItem = serde_json::from_value(serde_json::json!({
            "_objectType": "contentResource",
            "type": "audiobook",
            "id": {"_objectType": "universalMusicObjectId", "objectId": "x"},
            "name": "A book"
        }))
        .unwrap();
        assert_eq!(item.r#type, ContentType::Unknown);
        assert!(item.playable);
        assert!(!item.explicit);
    }
}
