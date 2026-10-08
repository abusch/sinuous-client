use std::fmt;

use serde::Deserialize;

use crate::groups::GroupCoordinatorChanged;

pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Searching for players on the network failed.
    #[error("failed to discover players")]
    Discovery(#[source] BoxError),
    /// Establishing the websocket connection (or learning the player's household) failed.
    #[error("failed to connect to player")]
    Connect(#[source] BoxError),
    /// The websocket connection is closed (or was closed while waiting for a reply).
    #[error("connection is closed")]
    ConnectionClosed,
    /// The player did not reply within the configured request timeout.
    #[error("request timed out")]
    Timeout,
    /// The player rejected the command.
    #[error("command failed: {0}")]
    Api(ApiError),
    /// The targeted group is not (or no longer) coordinated by the player this connection is
    /// connected to.
    ///
    /// Group- and player-scoped commands must be sent to the group coordinator's websocket. When
    /// the group has moved, [`GroupCoordinatorChanged::websocket_url`] says where to go instead.
    #[error("group coordinator changed ({:?})", .0.group_status)]
    GroupCoordinatorChanged(Box<GroupCoordinatorChanged>),
    /// The player sent a message we could not make sense of.
    #[error("invalid message from player")]
    Protocol(#[source] BoxError),
}

/// A `globalError` returned by a player when a command fails.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct ApiError {
    /// e.g. `ERROR_INVALID_PARAMETER`, `ERROR_UNSUPPORTED_COMMAND`...
    pub error_code: String,
    pub reason: Option<String>,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            Some(reason) => write!(f, "{} ({reason})", self.error_code),
            None => f.write_str(&self.error_code),
        }
    }
}
