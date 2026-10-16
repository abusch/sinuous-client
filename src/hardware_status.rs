//! The `hardwareStatus` namespace: the battery of portable players, and players' Wi-Fi
//! connection.
//!
//! Subscribing ([`Subscription::HardwareStatus`](crate::Subscription::HardwareStatus)) brings
//! [`WirelessNetworkStatus`] events from every player, and [`BatteryStatus`] ones from portable
//! players. It also brings Bluetooth, line-in and microphone switch events, which this crate
//! doesn't decode yet: they're delivered as
//! [`EventPayload::Unknown`](crate::EventPayload::Unknown).

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Deserializer};

use crate::{Error, PlayerHandle, protocol::NoParams};

const NAMESPACE: &str = "hardwareStatus";

impl PlayerHandle {
    /// Get the state of the player's battery.
    ///
    /// Players without a battery reject this with an [`Error::Api`] whose code is
    /// `ERROR_UNSUPPORTED_COMMAND`.
    pub async fn get_battery_status(&self) -> Result<BatteryStatus, Error> {
        self.request(NAMESPACE, "getBatteryStatus", &NoParams {})
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

/// A player's Wi-Fi connection. Only sent as an event: players don't allow reading it with a
/// command.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "_objectType", rename_all = "camelCase")]
pub struct WirelessNetworkStatus {
    /// The name of the network.
    #[serde(deserialize_with = "deserialize_ssid")]
    pub ssid: String,
    /// The access point's MAC address.
    pub bssid: Option<String>,
    /// In MHz, e.g. 2462 or 5320.
    pub frequency: Option<u32>,
    pub channel: Option<u32>,
    /// The signal strength as players report it: higher is better, e.g. 25 to 64. It isn't in
    /// dBm.
    pub rssi: Option<i32>,
    pub connection_health: ConnectionHealth,
    /// e.g. `STATION`.
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConnectionHealth {
    Excellent,
    Good,
    /// A value this crate doesn't know about yet, e.g. for a weaker connection.
    #[serde(other)]
    Unknown,
}

/// Players send SSIDs base64-encoded. Keep the text as is if it isn't valid base64.
fn deserialize_ssid<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let text = String::deserialize(deserializer)?;
    Ok(match BASE64.decode(&text) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => text,
    })
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
    fn decodes_wireless_network_status() {
        // From a Move on 2.4 GHz.
        let status: WirelessNetworkStatus = serde_json::from_value(json!({
            "_objectType": "wirelessNetworkStatus",
            "ssid": "YW14IG5ldHdvcms=",
            "frequency": 2462,
            "netmode": "NETMODE_STATION",
            "mode": "STATION",
            "connectionHealth": "EXCELLENT",
            "channel": 11,
            "bssid": "90:41:b2:06:3a:ff",
            "rssi": 45
        }))
        .unwrap();
        assert_eq!(
            status,
            WirelessNetworkStatus {
                ssid: "amx network".into(),
                bssid: Some("90:41:b2:06:3a:ff".into()),
                frequency: Some(2462),
                channel: Some(11),
                rssi: Some(45),
                connection_health: ConnectionHealth::Excellent,
                mode: Some("STATION".into()),
            }
        );
    }

    #[test]
    fn keeps_ssids_that_are_not_base64() {
        let status: WirelessNetworkStatus = serde_json::from_value(json!({
            "_objectType": "wirelessNetworkStatus",
            "ssid": "my network!",
            "connectionHealth": "POOR"
        }))
        .unwrap();
        assert_eq!(status.ssid, "my network!");
        assert_eq!(status.connection_health, ConnectionHealth::Unknown);
        assert_eq!(status.rssi, None);
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
