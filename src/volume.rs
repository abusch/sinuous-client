//! The `groupVolume` and `playerVolume` namespaces.

use serde::{Deserialize, Serialize};

use crate::{
    Connection, Error, GroupHandle, PlayerHandle,
    protocol::{NoParams, Target},
};

const GROUP_NAMESPACE: &str = "groupVolume";
const PLAYER_NAMESPACE: &str = "playerVolume";

/// The volume of a group or player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct Volume {
    /// 0 to 100.
    pub volume: u8,
    pub muted: bool,
    /// The volume can't be changed (e.g. line-out set to fixed volume).
    #[serde(default)]
    pub fixed: bool,
}

impl GroupHandle {
    /// Get the volume of the group as a whole.
    pub async fn get_volume(&self) -> Result<Volume, Error> {
        get_volume(&self.conn, GROUP_NAMESPACE, self.target()).await
    }

    /// Set the group volume (0-100). Player volumes are adjusted proportionally.
    pub async fn set_volume(&self, volume: u8) -> Result<(), Error> {
        set_volume(&self.conn, GROUP_NAMESPACE, self.target(), volume).await
    }

    /// Increase (positive) or decrease (negative) the group volume.
    pub async fn set_relative_volume(&self, delta: i8) -> Result<(), Error> {
        set_relative_volume(&self.conn, GROUP_NAMESPACE, self.target(), delta).await
    }

    pub async fn set_mute(&self, muted: bool) -> Result<(), Error> {
        set_mute(&self.conn, GROUP_NAMESPACE, self.target(), muted).await
    }
}

impl PlayerHandle {
    pub async fn get_volume(&self) -> Result<Volume, Error> {
        get_volume(&self.conn, PLAYER_NAMESPACE, self.target()).await
    }

    /// Set the player volume (0-100).
    pub async fn set_volume(&self, volume: u8) -> Result<(), Error> {
        set_volume(&self.conn, PLAYER_NAMESPACE, self.target(), volume).await
    }

    /// Increase (positive) or decrease (negative) the player volume.
    pub async fn set_relative_volume(&self, delta: i8) -> Result<(), Error> {
        set_relative_volume(&self.conn, PLAYER_NAMESPACE, self.target(), delta).await
    }

    pub async fn set_mute(&self, muted: bool) -> Result<(), Error> {
        set_mute(&self.conn, PLAYER_NAMESPACE, self.target(), muted).await
    }
}

async fn get_volume(
    conn: &Connection,
    namespace: &'static str,
    target: Target<'_>,
) -> Result<Volume, Error> {
    conn.request(namespace, "getVolume", target, &NoParams {})
        .await
}

async fn set_volume(
    conn: &Connection,
    namespace: &'static str,
    target: Target<'_>,
    volume: u8,
) -> Result<(), Error> {
    #[derive(Serialize)]
    struct Params {
        volume: u8,
    }

    conn.command(namespace, "setVolume", target, &Params { volume })
        .await
}

async fn set_relative_volume(
    conn: &Connection,
    namespace: &'static str,
    target: Target<'_>,
    volume_delta: i8,
) -> Result<(), Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        volume_delta: i8,
    }

    conn.command(
        namespace,
        "setRelativeVolume",
        target,
        &Params { volume_delta },
    )
    .await
}

async fn set_mute(
    conn: &Connection,
    namespace: &'static str,
    target: Target<'_>,
    muted: bool,
) -> Result<(), Error> {
    #[derive(Serialize)]
    struct Params {
        muted: bool,
    }

    conn.command(namespace, "setMute", target, &Params { muted })
        .await
}
