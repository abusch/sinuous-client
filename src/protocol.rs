//! Wire format of the websocket API.
//!
//! Every message, in either direction, is a JSON array of two objects: `[header, body]`.
//!
//! * Requests carry `namespace`, `command`, `cmdId` and a target (`householdId`, `groupId` or
//!   `playerId`) in the header; command parameters go in the body.
//! * Replies echo `cmdId` and carry `success` in the header; the body is the response object
//!   (or `{}` for commands that return nothing), or a `globalError` /
//!   `groupCoordinatorChanged` object when `success` is false.
//! * Events carry `name` in the header and the event object in the body.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error, GroupId, HouseholdId, PlayerId,
    error::ApiError,
    events::{Event, EventPayload},
    groups::GroupCoordinatorChanged,
};

/// What a command applies to. Serialized as a single `householdId`, `groupId` or `playerId` key.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum Target<'a> {
    #[serde(rename = "householdId")]
    Household(&'a HouseholdId),
    #[serde(rename = "groupId")]
    Group(&'a GroupId),
    #[serde(rename = "playerId")]
    Player(&'a PlayerId),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestHeader<'a> {
    pub namespace: &'static str,
    pub command: &'static str,
    pub cmd_id: &'a str,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target<'a>>,
}

/// Body for commands that take no parameters.
#[derive(Debug, Serialize)]
pub(crate) struct NoParams {}

pub(crate) fn encode_request(
    header: &RequestHeader<'_>,
    body: &impl Serialize,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&(header, body))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IncomingHeader {
    namespace: String,
    household_id: Option<HouseholdId>,
    group_id: Option<GroupId>,
    player_id: Option<PlayerId>,
    // Replies only.
    cmd_id: Option<String>,
    success: Option<bool>,
    // Events only.
    name: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Incoming {
    Reply {
        cmd_id: Option<String>,
        household_id: Option<HouseholdId>,
        result: Result<Value, Error>,
    },
    Event(Box<Event>),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProtocolError {
    #[error("malformed message")]
    Malformed(#[from] serde_json::Error),
    #[error("message is neither a reply nor an event")]
    UnknownKind,
    #[error("player did not report its household")]
    MissingHousehold,
    #[error("command failed with unexpected response {0}")]
    UnexpectedFailure(Value),
    #[error("failed to decode response to `{command}`")]
    Body {
        command: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

impl From<ProtocolError> for Error {
    fn from(e: ProtocolError) -> Self {
        Error::Protocol(Box::new(e))
    }
}

pub(crate) fn decode(text: &str) -> Result<Incoming, ProtocolError> {
    let (header, body): (IncomingHeader, Value) = serde_json::from_str(text)?;

    if let Some(name) = header.name {
        return Ok(Incoming::Event(Box::new(Event {
            namespace: header.namespace,
            name,
            household_id: header.household_id,
            group_id: header.group_id,
            player_id: header.player_id,
            payload: EventPayload::from_body(body),
        })));
    }

    let Some(success) = header.success else {
        return Err(ProtocolError::UnknownKind);
    };
    let result = if success {
        Ok(body)
    } else {
        Err(failure(body))
    };
    Ok(Incoming::Reply {
        cmd_id: header.cmd_id,
        household_id: header.household_id,
        result,
    })
}

/// Convert the body of an unsuccessful reply into an [`Error`].
fn failure(body: Value) -> Error {
    let decoded = match object_type(&body) {
        Some("globalError") => ApiError::deserialize(&body).map(Error::Api),
        Some("groupCoordinatorChanged") => GroupCoordinatorChanged::deserialize(&body)
            .map(|e| Error::GroupCoordinatorChanged(Box::new(e))),
        _ => return ProtocolError::UnexpectedFailure(body).into(),
    };
    decoded.unwrap_or_else(|_| ProtocolError::UnexpectedFailure(body).into())
}

pub(crate) fn object_type(body: &Value) -> Option<&str> {
    body.get("_objectType").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::groups::GroupStatus;

    #[test]
    fn encodes_request() {
        let group = GroupId::new("RINCON_1:42");
        let header = RequestHeader {
            namespace: "groupVolume",
            command: "setVolume",
            cmd_id: "7",
            target: Some(Target::Group(&group)),
        };
        let text = encode_request(&header, &json!({"volume": 20})).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            json!([
                {"namespace": "groupVolume", "command": "setVolume", "cmdId": "7", "groupId": "RINCON_1:42"},
                {"volume": 20}
            ])
        );
    }

    #[test]
    fn encodes_empty_body() {
        let household = HouseholdId::new("Sonos_1");
        let header = RequestHeader {
            namespace: "groups",
            command: "getGroups",
            cmd_id: "1",
            target: Some(Target::Household(&household)),
        };
        let text = encode_request(&header, &NoParams {}).unwrap();
        assert!(text.ends_with(",{}]"), "{text}");
    }

    #[test]
    fn encodes_request_without_target() {
        let header = RequestHeader {
            namespace: "groups",
            command: "getGroups",
            cmd_id: "0",
            target: None,
        };
        let text = encode_request(&header, &NoParams {}).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            json!([{"namespace": "groups", "command": "getGroups", "cmdId": "0"}, {}])
        );
    }

    #[test]
    fn decodes_successful_reply() {
        let text = r#"[{"namespace":"groupVolume","householdId":"Sonos_1","groupId":"RINCON_1:42","response":"getVolume","success":true,"type":"groupVolume","cmdId":"5"},{"_objectType":"groupVolume","volume":22,"muted":false,"fixed":false}]"#;
        let Incoming::Reply {
            cmd_id,
            household_id,
            result,
        } = decode(text).unwrap()
        else {
            panic!("expected a reply");
        };
        assert_eq!(cmd_id.as_deref(), Some("5"));
        assert_eq!(household_id, Some(HouseholdId::new("Sonos_1")));
        assert_eq!(result.unwrap()["volume"], 22);
    }

    #[test]
    fn decodes_subscribe_ack() {
        let text = r#"[{"namespace":"playback","householdId":"Sonos_1","groupId":"RINCON_1:42","response":"subscribe","success":true,"type":"none","cmdId":"s2"},{}]"#;
        let Incoming::Reply { result, .. } = decode(text).unwrap() else {
            panic!("expected a reply");
        };
        assert_eq!(result.unwrap(), json!({}));
    }

    #[test]
    fn decodes_global_error() {
        let text = r#"[{"namespace":"groups","householdId":"Sonos_1","response":"bogus","success":false,"type":"globalError","cmdId":"c3"},{"_objectType":"globalError","errorCode":"ERROR_UNSUPPORTED_COMMAND","reason":"bogus command is not supported."}]"#;
        let Incoming::Reply { result, .. } = decode(text).unwrap() else {
            panic!("expected a reply");
        };
        let Err(Error::Api(err)) = result else {
            panic!("expected an api error, got {result:?}");
        };
        assert_eq!(err.error_code, "ERROR_UNSUPPORTED_COMMAND");
        assert_eq!(
            err.reason.as_deref(),
            Some("bogus command is not supported.")
        );
    }

    #[test]
    fn decodes_group_coordinator_changed() {
        let text = r#"[{"namespace":"playback","householdId":"Sonos_1","groupId":"RINCON_2:1","response":"getPlaybackStatus","success":false,"type":"groupCoordinatorChanged","cmdId":"c3"},{"_objectType":"groupCoordinatorChanged","groupStatus":"GROUP_STATUS_MOVED","groupName":"Bedroom","websocketUrl":"wss:\/\/10.10.125.172:1443\/websocket\/api","playerId":"RINCON_2"}]"#;
        let Incoming::Reply { result, .. } = decode(text).unwrap() else {
            panic!("expected a reply");
        };
        let Err(Error::GroupCoordinatorChanged(changed)) = result else {
            panic!("expected group coordinator changed, got {result:?}");
        };
        assert_eq!(changed.group_status, GroupStatus::Moved);
        assert_eq!(
            changed.websocket_url.unwrap().as_str(),
            "wss://10.10.125.172:1443/websocket/api"
        );
        assert_eq!(changed.player_id, Some(PlayerId::new("RINCON_2")));
    }

    #[test]
    fn decodes_unexpected_failure_as_protocol_error() {
        let text =
            r#"[{"namespace":"x","success":false,"cmdId":"1"},{"_objectType":"somethingElse"}]"#;
        let Incoming::Reply { result, .. } = decode(text).unwrap() else {
            panic!("expected a reply");
        };
        assert!(matches!(result, Err(Error::Protocol(_))), "{result:?}");
    }

    #[test]
    fn decodes_event() {
        let text = r#"[{"namespace":"groupVolume","householdId":"Sonos_1","groupId":"RINCON_1:42","name":"groupVolume","type":"groupVolume"},{"_objectType":"groupVolume","volume":22,"muted":false,"fixed":false}]"#;
        let Incoming::Event(event) = decode(text).unwrap() else {
            panic!("expected an event");
        };
        assert_eq!(event.namespace, "groupVolume");
        assert_eq!(event.group_id, Some(GroupId::new("RINCON_1:42")));
        let EventPayload::GroupVolume(volume) = event.payload else {
            panic!("unexpected payload {:?}", event.payload);
        };
        assert_eq!(volume.volume, 22);
    }

    #[test]
    fn rejects_non_pair() {
        assert!(decode(r#"[{"namespace":"x"}]"#).is_err());
        assert!(decode(r#"{"namespace":"x"}"#).is_err());
    }
}
