//! The `homeTheater` namespace (soundbars).

use serde::Deserialize;

use crate::{Error, PlayerHandle, protocol::NoParams};

const NAMESPACE: &str = "homeTheater";

impl PlayerHandle {
    pub async fn get_home_theater_options(&self) -> Result<HomeTheaterOptions, Error> {
        self.conn
            .request(NAMESPACE, "getOptions", self.target(), &NoParams {})
            .await
    }
}

/// Home theater settings. Fields are `None` when the player doesn't support them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct HomeTheaterOptions {
    pub night_mode: Option<bool>,
    pub enhance_dialog: Option<bool>,
    pub enhance_dialog_level: Option<u32>,
    pub grouping_latency: Option<u32>,
    pub enable_true_room: Option<bool>,
    pub enable_virtual_height: Option<bool>,
}
