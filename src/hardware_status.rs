//! The `hardwareStatus` namespace: the battery of portable players.
//!
//! Subscribing ([`Subscription::HardwareStatus`](crate::Subscription::HardwareStatus)) also
//! brings Wi-Fi and Bluetooth status events, which this crate doesn't decode yet: they're
//! delivered as [`EventPayload::Unknown`](crate::EventPayload::Unknown).

use serde::Deserialize;

use crate::{Error, PlayerHandle, protocol::NoParams};

const NAMESPACE: &str = "hardwareStatus";

impl PlayerHandle {
    /// Get the state of the player's battery.
    ///
    /// Players without a battery reject this with an [`Error::Api`] whose code is
    /// `ERROR_UNSUPPORTED_COMMAND`.
    pub async fn get_battery_status(&self) -> Result<BatteryStatus, Error> {
        self.conn
            .request(NAMESPACE, "getBatteryStatus", self.target(), &NoParams {})
            .await
    }
}

/// The battery of a portable player.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct BatteryStatus {
    /// The charge shown to users, 0 to 100.
    pub battery_percentage: f64,
    /// The charge actually measured, which can differ from
    /// [`battery_percentage`](Self::battery_percentage), e.g. 91 when that says 100.
    pub raw_battery_percentage: Option<f64>,
    pub charging_state: ChargingState,
    /// In °C.
    pub battery_temperature: Option<f64>,
    #[serde(default)]
    pub valid_charger: ChargerValidity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ChargingState {
    #[serde(rename = "CHARGING")]
    Charging,
    /// e.g. when off its charger.
    #[serde(rename = "NOT_CHARGING")]
    NotCharging,
    /// A state this crate doesn't know about yet.
    #[serde(other)]
    Unknown,
}

/// Whether the player is plugged into a suitable charger.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ChargerValidity {
    #[serde(rename = "VALID")]
    Valid,
    #[serde(rename = "NOT_APPLICABLE")]
    NotApplicable,
    /// A value this crate doesn't know about yet.
    #[default]
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn decodes_battery_status() {
        // From a Roam 2 charging over USB-C.
        let status: BatteryStatus = serde_json::from_value(json!({
            "_objectType": "battery",
            "rawBatteryPercentage": 91.0,
            "batteryPercentage": 100.0,
            "chargingState": "CHARGING",
            "batteryTemperature": 28.0,
            "validCharger": "VALID"
        }))
        .unwrap();
        assert_eq!(
            status,
            BatteryStatus {
                battery_percentage: 100.0,
                raw_battery_percentage: Some(91.0),
                charging_state: ChargingState::Charging,
                battery_temperature: Some(28.0),
                valid_charger: ChargerValidity::Valid,
            }
        );
    }

    #[test]
    fn decodes_not_charging() {
        // From a Move lifted off its base.
        let status: BatteryStatus = serde_json::from_value(json!({
            "_objectType": "battery",
            "rawBatteryPercentage": 99.0,
            "batteryPercentage": 100.0,
            "chargingState": "NOT_CHARGING",
            "batteryTemperature": 22.0,
            "validCharger": "NOT_APPLICABLE"
        }))
        .unwrap();
        assert_eq!(status.charging_state, ChargingState::NotCharging);
        assert_eq!(status.valid_charger, ChargerValidity::NotApplicable);
    }

    #[test]
    fn keeps_unknown_states() {
        let status: BatteryStatus = serde_json::from_value(json!({
            "_objectType": "battery",
            "batteryPercentage": 42.5,
            "chargingState": "SOMETHING_NEW",
            "validCharger": "SOMETHING_ELSE"
        }))
        .unwrap();
        assert_eq!(status.battery_percentage, 42.5);
        assert_eq!(status.charging_state, ChargingState::Unknown);
        assert_eq!(status.valid_charger, ChargerValidity::Unknown);
        assert_eq!(status.raw_battery_percentage, None);
    }
}
