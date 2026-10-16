//! The `playbackExtended` namespace: a group's playback status and metadata together.
//!
//! One subscription ([`Subscription::PlaybackExtended`](crate::Subscription::PlaybackExtended))
//! instead of two (`Playback` and `PlaybackMetadata`), and the two parts always describe the same
//! moment.

use serde::Deserialize;

use crate::{
    Error, GroupHandle, playback::PlaybackStatus, playback_metadata::MetadataStatus,
    protocol::NoParams,
};

const NAMESPACE: &str = "playbackExtended";

impl GroupHandle {
    /// Get the group's playback status and metadata at once.
    pub async fn get_extended_playback_status(&self) -> Result<ExtendedPlaybackStatus, Error> {
        self.request(NAMESPACE, "getExtendedPlaybackStatus", &NoParams {})
            .await
    }
}

/// What [`GroupHandle::get_playback_status`] and [`GroupHandle::get_metadata_status`] return,
/// together.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct ExtendedPlaybackStatus {
    pub playback: PlaybackStatus,
    pub metadata: MetadataStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::PlaybackState;

    #[test]
    fn decodes_extended_playback_status() {
        // Trimmed down from a soundbar playing TV audio.
        let status: ExtendedPlaybackStatus = serde_json::from_str(
            r#"{"_objectType":"extendedPlaybackStatus","playback":{"_objectType":"playbackStatus","playbackState":"PLAYBACK_STATE_PLAYING","isDucking":false,"positionMillis":0,"previousPositionMillis":0,"playModes":{"_objectType":"playMode","repeat":false,"repeatOne":false,"shuffle":false,"crossfade":false},"availablePlaybackActions":{"_objectType":"playbackAction","canPlay":true,"canSkip":false,"canSkipBack":false,"canSkipToPrevious":false,"canSeek":false,"canPause":false,"canStop":false,"canRepeat":false,"canRepeatOne":false,"canCrossfade":false,"canShuffle":false},"sourceIsLanSwappable":false},"metadata":{"_objectType":"metadataStatus","container":{"_objectType":"container","name":"TV Audio","type":"linein.homeTheater.hdmi","id":{"_objectType":"universalMusicObjectId","objectId":"homeTheater-input"},"images":[],"htInputFormat":{"_objectType":"homeTheaterInputFormat","numGroundChannels":5,"numLFEChannels":1,"numHeightChannels":0,"streamDescription":"Dolby Digital Plus"}}}}"#,
        )
        .unwrap();
        assert_eq!(status.playback.playback_state, PlaybackState::Playing);
        assert!(status.playback.available_playback_actions.can_play);
        let container = status.metadata.container.unwrap();
        assert_eq!(container.name.as_deref(), Some("TV Audio"));
        assert!(status.metadata.current_item.is_none());
    }
}
