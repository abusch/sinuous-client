//! Local control of Sonos players over their websocket API.
//!
//! This is an unofficial client: it is not affiliated with or endorsed by Sonos, Inc. The local
//! API it uses is undocumented, and could change or stop working with any firmware update.
//!
//! The protocol mirrors the [Sonos Control API](https://docs.sonos.com/docs/control) (namespaces,
//! commands and objects), but is spoken directly to players on the local network:
//!
//! ```no_run
//! # use std::time::Duration;
//! # async fn example() -> Result<(), sinuous_client::Error> {
//! let players = sinuous_client::discover(Duration::from_secs(2)).await?;
//! let conn = players[0].connect().await?;
//! // Or, with a known address: sinuous_client::Connection::connect("10.10.190.82").await?
//! let groups = conn.get_groups().await?;
//! let group = conn.group(&groups.groups[0].id);
//! group.set_volume(20).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Unlike the cloud API, group commands must be sent to the group's coordinator and player
//! commands to the player itself. [`Household`] routes commands to the right player:
//!
//! ```no_run
//! # async fn example() -> Result<(), sinuous_client::Error> {
//! let household = sinuous_client::Household::connect("10.10.190.82").await?;
//! for group in &household.topology().groups {
//!     household.group(&group.id).await?.pause().await?;
//! }
//! # Ok(())
//! # }
//! ```

mod connection;
mod discovery;
mod error;
pub mod events;
pub mod favorites;
pub mod groups;
pub mod home_theater;
mod household;
mod ids;
pub mod playback;
pub mod playback_metadata;
pub mod playlists;
mod protocol;
#[cfg(test)]
mod test_support;
mod tls;
pub mod volume;

pub use connection::{ConnectOptions, Connection, DEFAULT_API_KEY, GroupHandle, PlayerHandle};
pub use discovery::{DiscoveredGroup, DiscoveredPlayer, discover};
pub use error::{ApiError, Error};
pub use events::{Event, EventPayload, Subscription};
pub use household::{Household, Topology};
pub use ids::{FavoriteId, GroupId, HouseholdId, PlayerId, PlaylistId};

// Check that the README example compiles.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
