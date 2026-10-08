//! The `playlists` namespace: Sonos playlists saved in the household.

use serde::{Deserialize, Serialize};

use crate::{
    Connection, Error, GroupHandle, PlaylistId, playback::LoadOptions, protocol::NoParams,
};

const NAMESPACE: &str = "playlists";

impl Connection {
    /// Get the household's Sonos playlists.
    pub async fn get_playlists(&self) -> Result<PlaylistsList, Error> {
        self.request(
            NAMESPACE,
            "getPlaylists",
            self.household_target(),
            &NoParams {},
        )
        .await
    }

    /// Get a playlist, including its tracks.
    pub async fn get_playlist(&self, playlist_id: &PlaylistId) -> Result<Playlist, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            playlist_id: &'a PlaylistId,
        }

        self.request(
            NAMESPACE,
            "getPlaylist",
            self.household_target(),
            &Params { playlist_id },
        )
        .await
    }
}

impl GroupHandle {
    /// Load a playlist into this group.
    pub async fn load_playlist(
        &self,
        playlist_id: &PlaylistId,
        options: &LoadOptions,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            playlist_id: &'a PlaylistId,
            #[serde(flatten)]
            options: &'a LoadOptions,
        }

        self.conn
            .command(
                NAMESPACE,
                "loadPlaylist",
                self.target(),
                &Params {
                    playlist_id,
                    options,
                },
            )
            .await
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct PlaylistsList {
    /// Changes whenever the list changes; see the `versionChanged` event.
    pub version: Option<String>,
    pub playlists: Vec<PlaylistSummary>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct PlaylistSummary {
    pub id: PlaylistId,
    pub name: String,
    pub r#type: Option<String>,
    pub track_count: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub r#type: Option<String>,
    #[serde(default)]
    pub tracks: Vec<PlaylistTrack>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct PlaylistTrack {
    pub name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}
