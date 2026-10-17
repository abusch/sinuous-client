//! The `playback` namespace: transport control for a group.

use serde::{Deserialize, Serialize};

use crate::{
    Error, GroupHandle, PlayerId, history::ContentType, playback_metadata::UniversalMusicObjectId,
    protocol::NoParams,
};

const NAMESPACE: &str = "playback";

impl GroupHandle {
    pub async fn get_playback_status(&self) -> Result<PlaybackStatus, Error> {
        self.request(NAMESPACE, "getPlaybackStatus", &NoParams {})
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

    /// Skip to the previous track, even when well into the current one.
    ///
    /// See [`GroupHandle::skip_back`] for a "back" button.
    pub async fn skip_to_previous_track(&self) -> Result<(), Error> {
        self.playback_command("skipToPreviousTrack").await
    }

    /// Restart the current track, or skip to the previous one within the first few seconds (3 to
    /// 4) of the current track: what a "back" button does.
    ///
    /// Like [`GroupHandle::skip_to_next_track`], this doesn't start playback. On the first track,
    /// it restarts it even though [`PlaybackActions::can_skip_back`] is false.
    pub async fn skip_back(&self) -> Result<(), Error> {
        self.playback_command("skipBack").await
    }

    /// Skip to the track at `track_number` in the group's queue, from 1.
    ///
    /// Track numbers are the queue positions that `GroupHandle::get_queue` gives (with the `upnp`
    /// feature), and the [`PlaybackStatus::item_id`] of a group playing from its queue.
    ///
    /// Like [`GroupHandle::skip_to_next_track`], this doesn't start playback: a playing group
    /// plays the track from its start, a paused or idle one stays so. Skipping past the end of
    /// the queue fails with `ERROR_PLAYBACK_NO_CONTENT`, and track 0 with
    /// `ERROR_INVALID_PARAMETER`.
    ///
    /// It only works while the group plays from its queue: during a Spotify Connect session, it
    /// succeeds but does nothing (or fails with `ERROR_DISALLOWED_BY_POLICY` when Spotify has
    /// nothing loaded).
    pub async fn skip_to_track(&self, track_number: u32) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params {
            track_number: u32,
        }

        self.command(NAMESPACE, "skipToTrack", &Params { track_number })
            .await
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

        self.command(
            NAMESPACE,
            "seek",
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

        self.command(
            NAMESPACE,
            "seekRelative",
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

        self.command(NAMESPACE, "setPlayModes", &Params { play_modes })
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

        self.command(
            NAMESPACE,
            "loadLineIn",
            &Params {
                device_id,
                play_on_completion,
            },
        )
        .await
    }

    /// Load content from a music service into the group, given its id and type, e.g. those of a
    /// [`HistoryItem`](crate::history::HistoryItem).
    ///
    /// Unlike [`GroupHandle::load_favorite`], this replaces the queue unless
    /// [`LoadOptions::action`] says otherwise. [`ContentType::Unknown`] can't be loaded, as the
    /// actual type isn't known.
    pub async fn load_content(
        &self,
        id: &UniversalMusicObjectId,
        content_type: ContentType,
        options: &LoadOptions,
    ) -> Result<(), Error> {
        /// [`UniversalMusicObjectId`] without its `_objectType`.
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Id<'a> {
            object_id: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            service_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            account_id: Option<&'a str>,
        }

        // Not the names `loadFavorite` and `loadPlaylist` use for the same options.
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Params<'a> {
            id: Id<'a>,
            r#type: ContentType,
            #[serde(skip_serializing_if = "Option::is_none")]
            queue_action: Option<LoadAction>,
            #[serde(skip_serializing_if = "Option::is_none")]
            playback_action: Option<&'static str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            play_modes: Option<PlayModesUpdate>,
        }

        let params = Params {
            id: Id {
                object_id: &id.object_id,
                service_id: id.service_id.as_deref(),
                account_id: id.account_id.as_deref(),
            },
            r#type: content_type,
            queue_action: options.action,
            playback_action: options
                .play_on_completion
                .unwrap_or(false)
                .then_some("PLAY"),
            play_modes: options.play_modes,
        };
        self.command(NAMESPACE, "loadContent", &params).await
    }

    async fn playback_command(&self, command: &'static str) -> Result<(), Error> {
        self.command(NAMESPACE, command, &NoParams {}).await
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
    /// Replace the queue with the content, and go to its first track.
    Replace,
    /// Add the content at the end of the queue.
    Append,
    /// Add the content at the end of the queue, and go to its first track.
    Insert,
    /// Add the content right after the current track.
    InsertNext,
}

/// Options for loading content (favorites, playlists, [content by id](GroupHandle::load_content))
/// into a group.
///
/// When `action` is `None`, favorites and playlists are appended to the queue, but content by id
/// replaces it.
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
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use serde_json::json;

    use super::*;
    use crate::{
        ConnectOptions, Connection, GroupId, HouseholdId,
        test_support::{fake_player, reply, url},
    };

    #[tokio::test]
    async fn skips_to_a_track() {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let (addr, _) = fake_player({
            let bodies = bodies.clone();
            move |header, body| {
                assert_eq!(header["namespace"], "playback");
                assert_eq!(header["command"], "skipToTrack");
                assert_eq!(header["groupId"], "RINCON_1:42");
                bodies.lock().unwrap().push(body.clone());
                Some(reply(header, true, json!({})))
            }
        })
        .await;
        let options = ConnectOptions::default()
            .request_timeout(Duration::from_millis(500))
            .household_id(HouseholdId::new("Sonos_1"));
        let conn = Connection::connect_to(&url(addr), options).await.unwrap();
        conn.group(&GroupId::new("RINCON_1:42"))
            .skip_to_track(3)
            .await
            .unwrap();
        assert_eq!(*bodies.lock().unwrap(), [json!({"trackNumber": 3})]);
    }

    #[tokio::test]
    async fn loads_content() {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let (addr, _) = fake_player({
            let bodies = bodies.clone();
            move |header, body| {
                assert_eq!(header["namespace"], "playback");
                assert_eq!(header["command"], "loadContent");
                bodies.lock().unwrap().push(body.clone());
                Some(reply(header, true, json!({})))
            }
        })
        .await;
        let options = ConnectOptions::default()
            .request_timeout(Duration::from_millis(500))
            .household_id(HouseholdId::new("Sonos_1"));
        let conn = Connection::connect_to(&url(addr), options).await.unwrap();
        let group = conn.group(&GroupId::new("RINCON_1:42"));
        let album = UniversalMusicObjectId {
            service_id: Some("12".into()),
            object_id: "spotify:album:1au1XG2AdakLtj9TPPNoSc".into(),
            account_id: Some("sn_77".into()),
        };
        group
            .load_content(&album, ContentType::Album, &LoadOptions::default())
            .await
            .unwrap();
        let sonos_playlist = UniversalMusicObjectId {
            service_id: Some("65435".into()),
            object_id: "0".into(),
            account_id: None,
        };
        let options = LoadOptions {
            action: Some(LoadAction::InsertNext),
            play_on_completion: Some(true),
            play_modes: Some(PlayModesUpdate {
                repeat: Some(true),
                ..Default::default()
            }),
        };
        group
            .load_content(&sonos_playlist, ContentType::Playlist, &options)
            .await
            .unwrap();
        assert_eq!(
            *bodies.lock().unwrap(),
            [
                json!({
                    "id": {
                        "objectId": "spotify:album:1au1XG2AdakLtj9TPPNoSc",
                        "serviceId": "12",
                        "accountId": "sn_77"
                    },
                    "type": "album"
                }),
                json!({
                    "id": {"objectId": "0", "serviceId": "65435"},
                    "type": "playlist",
                    "queueAction": "INSERT_NEXT",
                    "playbackAction": "PLAY",
                    "playModes": {"repeat": true}
                }),
            ]
        );
    }

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
