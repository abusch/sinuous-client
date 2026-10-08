//! The `favorites` namespace.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    Connection, Error, FavoriteId, GroupHandle,
    playback::LoadOptions,
    playback_metadata::{Service, UniversalMusicObjectId},
    protocol::NoParams,
};

const NAMESPACE: &str = "favorites";

impl Connection {
    /// Get the household's Sonos favorites.
    pub async fn get_favorites(&self) -> Result<FavoritesList, Error> {
        self.request(
            NAMESPACE,
            "getFavorites",
            self.household_target(),
            &NoParams {},
        )
        .await
    }
}

impl GroupHandle {
    /// Load a favorite into this group.
    pub async fn load_favorite(
        &self,
        favorite_id: &FavoriteId,
        options: &LoadOptions,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            favorite_id: &'a FavoriteId,
            #[serde(flatten)]
            options: &'a LoadOptions,
        }

        self.conn
            .command(
                NAMESPACE,
                "loadFavorite",
                self.target(),
                &Params {
                    favorite_id,
                    options,
                },
            )
            .await
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct FavoritesList {
    /// Changes whenever the list changes; see the `versionChanged` event.
    pub version: Option<String>,
    pub items: Vec<Favorite>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Favorite {
    pub id: FavoriteId,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<Url>,
    pub service: Option<Service>,
    pub resource: Option<FavoriteResource>,
}

/// The content a favorite points to.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct FavoriteResource {
    /// e.g. `PLAYLIST`, `ALBUM`, `TRACK`, `STATION`...
    pub r#type: Option<String>,
    pub id: Option<UniversalMusicObjectId>,
    pub name: Option<String>,
    #[serde(default)]
    pub explicit: bool,
}
