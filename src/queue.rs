//! The tracks in a group's queue.
//!
//! The websocket API only tells what's playing and what's next
//! ([`GroupHandle::get_metadata_status`]), so the queue is read from the players' UPnP interface
//! instead, over plain HTTP. This needs the `upnp` feature.
//!
//! This is the group's Sonos queue. Content played some other way doesn't go through it, e.g.
//! from the Spotify app (Spotify Connect), line-in, TV audio or a cloud queue: the queue keeps
//! whatever was last played from it, which is no longer what's playing. Use
//! [`Queue::is_current`] to tell.

use std::time::Duration;

use url::Url;

use crate::{
    Error, GroupHandle,
    playback::PlaybackStatus,
    playback_metadata::ImageUrl,
    upnp::{self, CONTENT_DIRECTORY, UpnpError},
};

const QUEUE_ID: &str = "Q:0";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const UPNP_NS: &str = "urn:schemas-upnp-org:metadata-1-0/upnp/";
const DIDL_NS: &str = "urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/";

impl GroupHandle {
    /// Get all the tracks in the group's queue.
    ///
    /// The queue may not be what the group is playing: see [`Queue::is_current`].
    ///
    /// Like other group commands, this must be sent through the group coordinator's connection
    /// (which [`Household::group`](crate::Household::group) does): other players return their own
    /// queue, which isn't the one being played.
    pub async fn get_queue(&self) -> Result<Queue, Error> {
        let control_url = CONTENT_DIRECTORY.control_url(self.conn.websocket_url())?;
        let timeout = self.conn.request_timeout();
        let mut queue = browse(&control_url, 0, 0, timeout).await?;
        // Players may return fewer tracks than asked for: get the rest. If the queue changes in
        // the meantime, start over so as not to mix two versions.
        while queue.items.len() < queue.total as usize {
            let page = browse(&control_url, queue.items.len() as u32, 0, timeout).await?;
            if page.version != queue.version {
                queue = browse(&control_url, 0, 0, timeout).await?;
            } else if page.items.is_empty() {
                break;
            } else {
                queue.items.extend(page.items);
                queue.total = page.total;
            }
        }
        Ok(queue)
    }

    /// Get up to `count` tracks of the group's queue, starting with the one at index `start`
    /// (from 0). A `count` of 0 means as many as the player is willing to return.
    ///
    /// See [`GroupHandle::get_queue`].
    pub async fn get_queue_page(&self, start: u32, count: u32) -> Result<Queue, Error> {
        let control_url = CONTENT_DIRECTORY.control_url(self.conn.websocket_url())?;
        browse(&control_url, start, count, self.conn.request_timeout()).await
    }
}

/// Some or all of the tracks in a group's queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub items: Vec<QueueEntry>,
    /// The number of tracks in the whole queue.
    pub total: u32,
    /// Changes whenever the queue does. The same as [`PlaybackStatus::queue_version`] while the
    /// group plays from its queue, so `playbackStatus` events tell when the queue needs reading
    /// again.
    pub version: String,
}

impl Queue {
    /// Whether the group is playing from this queue, according to its playback `status`.
    ///
    /// When it isn't, e.g. when playing from the Spotify app, [`PlaybackStatus::queue_version`]
    /// is unset (or belongs to another queue), and the queue still holds whatever was last
    /// played from it. [`GroupHandle::get_metadata_status`] then tells what's playing and what's
    /// next.
    pub fn is_current(&self, status: &PlaybackStatus) -> bool {
        status.queue_version.as_deref() == Some(self.version.as_str())
    }
}

/// A track in a group's queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    /// The track's position in the queue, from 1.
    ///
    /// While the group plays from its queue ([`Queue::is_current`]), [`PlaybackStatus::item_id`]
    /// is the current track's position, e.g. `"3"`.
    pub position: u32,
    pub name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Usually relative to the player: see [`ImageUrl::resolve`].
    pub image_url: Option<ImageUrl>,
    pub duration_millis: Option<u64>,
    /// The URI the player plays the track from, e.g. `x-sonos-spotify:spotify%3atrack%3a...`.
    pub uri: Option<String>,
}

async fn browse(
    control_url: &Url,
    start: u32,
    count: u32,
    timeout: Duration,
) -> Result<Queue, Error> {
    let start = start.to_string();
    let count = count.to_string();
    let args = [
        ("ObjectID", QUEUE_ID),
        ("BrowseFlag", "BrowseDirectChildren"),
        ("Filter", "*"),
        ("StartingIndex", start.as_str()),
        ("RequestedCount", count.as_str()),
        ("SortCriteria", ""),
    ];
    let output = upnp::call(&CONTENT_DIRECTORY, control_url, "Browse", &args, timeout).await?;
    let arg = |name: &'static str| output.get(name).ok_or(UpnpError::Invalid(name));
    Ok(Queue {
        items: parse_didl(arg("Result")?)?,
        total: arg("TotalMatches")?
            .parse()
            .map_err(|_| UpnpError::Invalid("TotalMatches"))?,
        version: arg("UpdateID")?.clone(),
    })
}

/// Parse the DIDL-Lite document listing the queue's tracks.
fn parse_didl(text: &str) -> Result<Vec<QueueEntry>, UpnpError> {
    let doc = roxmltree::Document::parse(text)?;
    doc.root_element()
        .children()
        .filter(|node| node.has_tag_name((DIDL_NS, "item")))
        .map(parse_item)
        .collect()
}

fn parse_item(item: roxmltree::Node<'_, '_>) -> Result<QueueEntry, UpnpError> {
    let child = |ns: &str, name: &str| item.children().find(|node| node.has_tag_name((ns, name)));
    let text = |ns: &str, name: &str| {
        child(ns, name)
            .and_then(|node| node.text())
            .map(str::to_owned)
    };
    let position = item
        .attribute("id")
        .and_then(|id| id.strip_prefix(QUEUE_ID)?.strip_prefix('/')?.parse().ok())
        .ok_or(UpnpError::Invalid("item id"))?;
    let res = child(DIDL_NS, "res");
    Ok(QueueEntry {
        position,
        name: text(DC_NS, "title"),
        artist: text(DC_NS, "creator"),
        album: text(UPNP_NS, "album"),
        image_url: text(UPNP_NS, "albumArtURI").map(ImageUrl),
        duration_millis: res
            .and_then(|res| res.attribute("duration"))
            .and_then(parse_duration_millis),
        uri: res.and_then(|res| res.text()).map(str::to_owned),
    })
}

/// Parse a duration like `0:05:24` or `1:02:03.500`.
fn parse_duration_millis(text: &str) -> Option<u64> {
    let mut parts = text.splitn(3, ':');
    let (hours, minutes, seconds) = (parts.next()?, parts.next()?, parts.next()?);
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    // Also makes slicing it by bytes safe.
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Only keep milliseconds.
    let digits = &fraction[..fraction.len().min(3)];
    let millis = match digits.len() {
        0 => 0,
        n => digits.parse::<u64>().ok()? * 10u64.pow(3 - n as u32),
    };
    let seconds = hours
        .parse::<u64>()
        .ok()?
        .checked_mul(3600)?
        .checked_add(minutes.parse::<u64>().ok()?.checked_mul(60)?)?
        .checked_add(seconds.parse().ok()?)?;
    seconds.checked_mul(1000)?.checked_add(millis)
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{Arc, Mutex},
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    /// A trimmed down response from a player.
    const RESPONSE: &str = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/"><s:Body><u:BrowseResponse xmlns:u="urn:schemas-upnp-org:service:ContentDirectory:1"><Result>&lt;DIDL-Lite xmlns:dc=&quot;http://purl.org/dc/elements/1.1/&quot; xmlns:upnp=&quot;urn:schemas-upnp-org:metadata-1-0/upnp/&quot; xmlns:r=&quot;urn:schemas-rinconnetworks-com:metadata-1-0/&quot; xmlns=&quot;urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/&quot;&gt;&lt;item id=&quot;Q:0/1&quot; parentID=&quot;Q:0&quot; restricted=&quot;true&quot;&gt;&lt;res protocolInfo=&quot;sonos.com-spotify:*:audio/x-spotify:*&quot; duration=&quot;0:05:24&quot;&gt;x-sonos-spotify:spotify%3atrack%3a5zGEoeJ8mmWBTHh4GOgBSS?sid=12&amp;amp;flags=8232&amp;amp;sn=77&lt;/res&gt;&lt;upnp:albumArtURI&gt;/getaa?s=1&amp;amp;u=x-sonos-spotify%3aspotify%253atrack%253a5zGEoeJ8mmWBTHh4GOgBSS%3fsid%3d12%26flags%3d8232%26sn%3d77&lt;/upnp:albumArtURI&gt;&lt;dc:title&gt;Escape&lt;/dc:title&gt;&lt;upnp:class&gt;object.item.audioItem.musicTrack&lt;/upnp:class&gt;&lt;dc:creator&gt;Boucle Infinie&lt;/dc:creator&gt;&lt;upnp:album&gt;Summit&lt;/upnp:album&gt;&lt;/item&gt;&lt;item id=&quot;Q:0/2&quot; parentID=&quot;Q:0&quot; restricted=&quot;true&quot;&gt;&lt;dc:title&gt;Contemplation&lt;/dc:title&gt;&lt;/item&gt;&lt;/DIDL-Lite&gt;</Result><NumberReturned>2</NumberReturned><TotalMatches>8</TotalMatches><UpdateID>4</UpdateID></u:BrowseResponse></s:Body></s:Envelope>"#;

    /// Serve HTTP requests with `respond(body)`, which returns the status line and body to send.
    async fn fake_upnp_player(
        respond: impl Fn(&str) -> (&'static str, String) + Send + 'static,
    ) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                // Players close the connection after each response, so read until the end of
                // the request's body, as given by its `Content-Length`.
                let mut request = Vec::new();
                let mut buf = [0; 4096];
                let body = loop {
                    let n = stream.read(&mut buf).await.unwrap();
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .and_then(|length| length.parse::<usize>().ok())
                            .unwrap_or(0);
                        if body.len() >= length {
                            assert!(head.contains(
                                r#"soapaction: "urn:schemas-upnp-org:service:ContentDirectory:1#Browse""#
                            ));
                            break body.to_owned();
                        }
                    }
                };
                let (status, response) = respond(&body);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        addr
    }

    fn control_url(addr: SocketAddr) -> Url {
        Url::parse(&format!(
            "http://{addr}/MediaServer/ContentDirectory/Control"
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn browses_the_queue() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let addr = fake_upnp_player({
            let requests = requests.clone();
            move |body| {
                requests.lock().unwrap().push(body.to_owned());
                ("200 OK", RESPONSE.to_owned())
            }
        })
        .await;
        let queue = browse(&control_url(addr), 3, 2, Duration::from_secs(1))
            .await
            .unwrap();

        let request = requests.lock().unwrap().pop().unwrap();
        assert!(
            request.contains("<ObjectID>Q:0</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>*</Filter><StartingIndex>3</StartingIndex><RequestedCount>2</RequestedCount>"),
            "{request}"
        );
        assert_eq!(queue.total, 8);
        assert_eq!(queue.version, "4");
        assert_eq!(
            queue.items[0],
            QueueEntry {
                position: 1,
                name: Some("Escape".into()),
                artist: Some("Boucle Infinie".into()),
                album: Some("Summit".into()),
                image_url: Some(ImageUrl("/getaa?s=1&u=x-sonos-spotify%3aspotify%253atrack%253a5zGEoeJ8mmWBTHh4GOgBSS%3fsid%3d12%26flags%3d8232%26sn%3d77".into())),
                duration_millis: Some(324_000),
                uri: Some("x-sonos-spotify:spotify%3atrack%3a5zGEoeJ8mmWBTHh4GOgBSS?sid=12&flags=8232&sn=77".into()),
            }
        );
        assert_eq!(queue.items[1].position, 2);
        assert_eq!(queue.items[1].name.as_deref(), Some("Contemplation"));
        assert_eq!(queue.items[1].duration_millis, None);
    }

    #[tokio::test]
    async fn reports_upnp_faults() {
        let addr = fake_upnp_player(|_| {
            (
                "500 Internal Server Error",
                r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns="urn:schemas-upnp-org:control-1-0"><errorCode>701</errorCode></UPnPError></detail></s:Fault></s:Body></s:Envelope>"#.to_owned(),
            )
        })
        .await;
        let err = browse(&control_url(addr), 0, 0, Duration::from_secs(1))
            .await
            .unwrap_err();
        let Error::Upnp(source) = &err else {
            panic!("unexpected error {err:?}");
        };
        assert_eq!(source.to_string(), "player returned UPnP error 701");
    }

    #[tokio::test]
    async fn rejects_malformed_responses() {
        let addr =
            fake_upnp_player(|_| ("200 OK", RESPONSE.replace("TotalMatches", "Total"))).await;
        let err = browse(&control_url(addr), 0, 0, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Protocol(_)), "{err:?}");
    }

    #[test]
    fn is_current_when_versions_match() {
        let queue = Queue {
            items: Vec::new(),
            total: 8,
            version: "4".into(),
        };
        let status = |queue_version: Option<&str>| -> PlaybackStatus {
            serde_json::from_value(serde_json::json!({
                "_objectType": "playbackStatus",
                "playbackState": "PLAYBACK_STATE_PLAYING",
                "queueVersion": queue_version,
            }))
            .unwrap()
        };
        assert!(queue.is_current(&status(Some("4"))));
        // The queue changed since it was read.
        assert!(!queue.is_current(&status(Some("5"))));
        // Playing from elsewhere, e.g. Spotify Connect.
        assert!(!queue.is_current(&status(None)));
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration_millis("0:05:24"), Some(324_000));
        assert_eq!(parse_duration_millis("1:02:03.5"), Some(3_723_500));
        assert_eq!(parse_duration_millis("0:00:01.123456"), Some(1_123));
        assert_eq!(parse_duration_millis("NOT_IMPLEMENTED"), None);
        assert_eq!(parse_duration_millis("0:05"), None);
        assert_eq!(parse_duration_millis("0:00:01.éé"), None);
        assert_eq!(parse_duration_millis("0:00:01.5x"), None);
        assert_eq!(parse_duration_millis("99999999999999999:00:00"), None);
        assert_eq!(parse_duration_millis(&format!("0:00:{}", u64::MAX)), None);
    }

    #[test]
    fn parses_an_empty_queue() {
        let didl = r#"<DIDL-Lite xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/" xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/"></DIDL-Lite>"#;
        assert_eq!(parse_didl(didl).unwrap(), []);
    }
}
