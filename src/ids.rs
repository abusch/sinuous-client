//! Strongly-typed identifiers used by the Sonos API.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_owned())
            }
        }
    };
}

id_type!(
    /// Identifies a Sonos household, e.g. `Sonos_FVGVbNxG94Pbng2LLMm8zdSVuT.nErh-aF_Y_qPBGkAza3J`.
    HouseholdId
);
id_type!(
    /// Identifies a group, e.g. `RINCON_74CA6062DF3601400:3739271259`.
    GroupId
);
id_type!(
    /// Identifies a player or device, e.g. `RINCON_74CA6062DF3601400`.
    PlayerId
);
id_type!(
    /// Identifies a Sonos favorite.
    FavoriteId
);
id_type!(
    /// Identifies a Sonos playlist.
    PlaylistId
);
