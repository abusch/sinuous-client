//! Requests to the players' UPnP interface: SOAP over plain HTTP, on port 1400.
//!
//! Some things are only available there, such as the contents of a group's queue.

use std::{collections::HashMap, fmt::Write as _, time::Duration};

use url::Url;

use crate::{Error, protocol::ProtocolError};

const PORT: u16 = 1400;
const SOAP_ENVELOPE_NS: &str = "http://schemas.xmlsoap.org/soap/envelope/";

/// A UPnP service of a player.
pub(crate) struct Service {
    /// Path of the service's control endpoint.
    control_path: &'static str,
    urn: &'static str,
}

pub(crate) const CONTENT_DIRECTORY: Service = Service {
    control_path: "/MediaServer/ContentDirectory/Control",
    urn: "urn:schemas-upnp-org:service:ContentDirectory:1",
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum UpnpError {
    #[error("HTTP request failed")]
    Http(#[from] reqwest::Error),
    #[error("player returned HTTP status {0}")]
    Status(reqwest::StatusCode),
    #[error("player returned UPnP error {code}")]
    Fault { code: String },
    #[error("malformed XML")]
    Xml(#[from] roxmltree::Error),
    #[error("missing or invalid `{0}` in UPnP response")]
    Invalid(&'static str),
}

impl From<UpnpError> for Error {
    fn from(e: UpnpError) -> Self {
        match e {
            UpnpError::Http(e) if e.is_timeout() => Error::Timeout,
            UpnpError::Http(_) | UpnpError::Status(_) | UpnpError::Fault { .. } => {
                Error::Upnp(Box::new(e))
            }
            UpnpError::Xml(_) | UpnpError::Invalid(_) => ProtocolError::Upnp(e).into(),
        }
    }
}

impl Service {
    /// The URL of this service on the player with the given websocket URL.
    pub(crate) fn control_url(&self, websocket_url: &Url) -> Result<Url, Error> {
        let mut url = websocket_url.clone();
        url.set_scheme("http")
            .and_then(|()| url.set_port(Some(PORT)))
            .map_err(|()| UpnpError::Invalid("websocket URL"))?;
        url.set_path(self.control_path);
        url.set_query(None);
        Ok(url)
    }
}

/// Call `action` on the service at `control_url`, and return the response's output arguments.
pub(crate) async fn call(
    service: &Service,
    control_url: &Url,
    action: &str,
    args: &[(&str, &str)],
    timeout: Duration,
) -> Result<HashMap<String, String>, UpnpError> {
    let response = reqwest::Client::new()
        .post(control_url.clone())
        .header("Content-Type", r#"text/xml; charset="utf-8""#)
        .header("SOAPACTION", format!(r#""{}#{action}""#, service.urn))
        .body(envelope(service, action, args))
        .timeout(timeout)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(
            fault_code(&text).map_or(UpnpError::Status(status), |code| UpnpError::Fault { code })
        );
    }
    output_arguments(&text, action)
}

fn envelope(service: &Service, action: &str, args: &[(&str, &str)]) -> String {
    let mut body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="{SOAP_ENVELOPE_NS}" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:{action} xmlns:u="{}">"#,
        service.urn
    );
    for (name, value) in args {
        let _ = write!(body, "<{name}>{}</{name}>", escape(value));
    }
    let _ = write!(body, "</u:{action}></s:Body></s:Envelope>");
    body
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The children of the `<{action}Response>` element, by name.
fn output_arguments(text: &str, action: &str) -> Result<HashMap<String, String>, UpnpError> {
    let doc = roxmltree::Document::parse(text)?;
    let response_name = format!("{action}Response");
    let response = doc
        .descendants()
        .find(|node| node.tag_name().name() == response_name)
        .ok_or(UpnpError::Invalid("response element"))?;
    Ok(response
        .children()
        .filter(|node| node.is_element())
        .map(|node| {
            let name = node.tag_name().name().to_owned();
            (name, node.text().unwrap_or_default().to_owned())
        })
        .collect())
}

/// The `errorCode` of a SOAP fault, if `text` is one.
fn fault_code(text: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(text).ok()?;
    let code = doc
        .descendants()
        .find(|node| node.tag_name().name() == "errorCode")?;
    code.text().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_url_points_at_the_upnp_port() {
        let ws = Url::parse("wss://10.10.107.230:1443/websocket/api").unwrap();
        assert_eq!(
            CONTENT_DIRECTORY.control_url(&ws).unwrap().as_str(),
            "http://10.10.107.230:1400/MediaServer/ContentDirectory/Control"
        );
        let ws = Url::parse("wss://[fe80::1]:1443/websocket/api").unwrap();
        assert_eq!(
            CONTENT_DIRECTORY.control_url(&ws).unwrap().as_str(),
            "http://[fe80::1]:1400/MediaServer/ContentDirectory/Control"
        );
    }

    #[test]
    fn envelope_escapes_arguments() {
        let body = envelope(&CONTENT_DIRECTORY, "Browse", &[("ObjectID", "Q:0&<x>")]);
        assert!(
            body.contains(r#"<u:Browse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1"><ObjectID>Q:0&amp;&lt;x&gt;</ObjectID></u:Browse>"#),
            "{body}"
        );
    }

    #[test]
    fn reads_output_arguments() {
        let text = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><u:BrowseResponse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1"><Result>&lt;DIDL-Lite/&gt;</Result><NumberReturned>0</NumberReturned></u:BrowseResponse></s:Body></s:Envelope>"#;
        let args = output_arguments(text, "Browse").unwrap();
        assert_eq!(args["Result"], "<DIDL-Lite/>");
        assert_eq!(args["NumberReturned"], "0");
        assert!(matches!(
            output_arguments(text, "GetPositionInfo"),
            Err(UpnpError::Invalid(_))
        ));
    }

    #[test]
    fn reads_fault_codes() {
        let text = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>701</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>"#;
        assert_eq!(fault_code(text).as_deref(), Some("701"));
        assert_eq!(fault_code("not xml"), None);
    }
}
