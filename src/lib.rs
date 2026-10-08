//! Local control of Sonos players over their websocket API.
//!
//! The protocol mirrors the [Sonos Control API](https://docs.sonos.com/docs/control) (namespaces,
//! commands and objects), but is spoken directly to players on the local network:
//!
//! ```no_run
//! # use std::time::Duration;
//! # async fn example() -> Result<(), sonos_ws::Error> {
//! let players = sonos_ws::discover(Duration::from_secs(2)).await?;
//! let conn = players[0].connect().await?;
//! // Or, with a known address: sonos_ws::Connection::connect("10.10.190.82").await?
//! let groups = conn.get_groups().await?;
//! let group = conn.group(&groups.groups[0].id);
//! group.set_volume(20).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Unlike the cloud API, group- and player-scoped commands must be sent to the player that
//! coordinates the group (see [`Error::GroupCoordinatorChanged`]).

mod connection;
mod discovery;
mod error;
pub mod events;
pub mod favorites;
pub mod groups;
pub mod home_theater;
mod ids;
pub mod playback;
pub mod playback_metadata;
pub mod playlists;
mod protocol;
mod tls;
pub mod volume;

pub use connection::{ConnectOptions, Connection, DEFAULT_API_KEY, GroupHandle, PlayerHandle};
pub use discovery::{DiscoveredGroup, DiscoveredPlayer, discover};
pub use error::{ApiError, Error};
pub use events::{Event, EventPayload, Subscription};
pub use ids::{FavoriteId, GroupId, HouseholdId, PlayerId, PlaylistId};
