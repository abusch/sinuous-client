//! The `playback` namespace: transport control for a group.

use serde::{Deserialize, Serialize};

use crate::{Error, GroupHandle, PlayerId, protocol::NoParams};

const NAMESPACE: &str = "playback";

impl GroupHandle {
    pub async fn get_playback_status(&self) -> Result<PlaybackStatus, Error> {
        self.conn
            .request(NAMESPACE, "getPlaybackStatus", self.target(), &NoParams {})
            .await
    }

    pub async fn play(&self) -> Result<(), Error> {
        self.playback_command("play").await
    }

    pub async fn pause(&self) -> Result<(), Error> {
        self.playback_command("pause").await
    }

    pub async fn toggle_play_pause(&self) -> Result<(), Error> {
        self.playback_command("togglePlayPause").await
    }

    pub async fn skip_to_next_track(&self) -> Result<(), Error> {
        self.playback_command("skipToNextTrack").await
    }

    pub async fn skip_to_previous_track(&self) -> Result<(), Error> {
        self.playback_command("skipToPreviousTrack").await
    }

    /// Seek to an absolute position in the current track.
    ///
    /// If `item_id` is given, the command only applies if it is still the current item.
    pub async fn seek(&self, position_millis: u64, item_id: Option<&str>) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            position_millis: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            item_id: Option<&'a str>,
        }

        self.conn
            .command(
                NAMESPACE,
                "seek",
                self.target(),
                &Params {
                    position_millis,
                    item_id,
                },
            )
            .await
    }

    /// Seek forward (positive) or backward (negative) relative to the current position.
    ///
    /// If `item_id` is given, the command only applies if it is still the current item.
    pub async fn seek_relative(
        &self,
        delta_millis: i64,
        item_id: Option<&str>,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            delta_millis: i64,
            #[serde(skip_serializing_if = "Option::is_none")]
            item_id: Option<&'a str>,
        }

        self.conn
            .command(
                NAMESPACE,
                "seekRelative",
                self.target(),
                &Params {
                    delta_millis,
                    item_id,
                },
            )
            .await
    }

    /// Change some of the play modes. Modes left as `None` are unchanged.
    pub async fn set_play_modes(&self, play_modes: &PlayModesUpdate) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            play_modes: &'a PlayModesUpdate,
        }

        self.conn
            .command(
                NAMESPACE,
                "setPlayModes",
                self.target(),
                &Params { play_modes },
            )
            .await
    }

    /// Switch the group to the line-in of `device_id` (or the coordinator's if `None`).
    pub async fn load_line_in(
        &self,
        device_id: Option<&PlayerId>,
        play_on_completion: bool,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            device_id: Option<&'a PlayerId>,
            play_on_completion: bool,
        }

        self.conn
            .command(
                NAMESPACE,
                "loadLineIn",
                self.target(),
                &Params {
                    device_id,
                    play_on_completion,
                },
            )
            .await
    }

    async fn playback_command(&self, command: &'static str) -> Result<(), Error> {
        self.conn
            .command(NAMESPACE, command, self.target(), &NoParams {})
            .await
    }
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
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct PlaybackStatus {
    pub playback_state: PlaybackState,
    #[serde(default)]
    pub is_ducking: bool,
    pub queue_version: Option<String>,
    pub item_id: Option<String>,
    pub position_millis: Option<u64>,
    pub previous_item_id: Option<String>,
    pub previous_position_millis: Option<u64>,
    #[serde(default)]
    pub play_modes: PlayModes,
    #[serde(default)]
    pub available_playback_actions: PlaybackActions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase", default)]
pub struct PlayModes {
    pub repeat: bool,
    pub repeat_one: bool,
    pub shuffle: bool,
    pub crossfade: bool,
}

/// A partial update of [`PlayModes`], for [`GroupHandle::set_play_modes`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayModesUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_one: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shuffle: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crossfade: Option<bool>,
}

/// Which actions the current source supports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase", default)]
pub struct PlaybackActions {
    pub can_play: bool,
    pub can_pause: bool,
    pub can_stop: bool,
    pub can_skip: bool,
    pub can_skip_back: bool,
    pub can_skip_to_previous: bool,
    pub can_seek: bool,
    pub can_repeat: bool,
    pub can_repeat_one: bool,
    pub can_crossfade: bool,
    pub can_shuffle: bool,
}

/// Sent as a `playbackError` event when playback fails.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct PlaybackError {
    pub error_code: String,
    pub reason: Option<String>,
    pub item_id: Option<String>,
}

/// How loaded content is combined with the current queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LoadAction {
    Replace,
    Append,
    Insert,
    InsertNext,
}

/// Options for loading content (favorites, playlists) into a group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<LoadAction>,
    /// Start playing once the content is loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_on_completion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_modes: Option<PlayModesUpdate>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_playback_status() {
        let status: PlaybackStatus = serde_json::from_str(
            r#"{"_objectType":"playbackStatus","availablePlaybackActions":{"_objectType":"playbackAction","canCrossfade":false,"canPause":true,"canPlay":true,"canRepeat":false,"canRepeatOne":false,"canSeek":false,"canShuffle":false,"canSkip":false,"canSkipBack":true,"canSkipToPrevious":true,"canStop":true},"isDucking":false,"itemId":"54b8a7ef@1791016974084234468","playModes":{"_objectType":"playMode","crossfade":false,"repeat":true,"repeatOne":false,"shuffle":false},"playbackState":"PLAYBACK_STATE_PAUSED","positionMillis":216000,"previousItemId":"54b8a7ef@1791016974084234468","previousPositionMillis":216000,"queueVersion":"1791436516.9542508","sourceIsLanSwappable":false}"#,
        )
        .unwrap();
        assert_eq!(status.playback_state, PlaybackState::Paused);
        assert_eq!(status.position_millis, Some(216000));
        assert!(status.play_modes.repeat);
        assert!(status.available_playback_actions.can_pause);
        assert!(!status.available_playback_actions.can_seek);
    }

    #[test]
    fn load_options_only_serialize_what_is_set() {
        let options = LoadOptions {
            action: Some(LoadAction::InsertNext),
            play_modes: Some(PlayModesUpdate {
                shuffle: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            serde_json::json!({"action": "INSERT_NEXT", "playModes": {"shuffle": true}})
        );
    }
}
