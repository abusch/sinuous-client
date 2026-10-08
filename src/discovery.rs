//! Finding players on the local network with SSDP.
//!
//! Players answer M-SEARCH requests for `urn:smartspeaker-audio:service:SpeakerGroup:1` with
//! everything needed to use the websocket API:
//!
//! ```text
//! HTTP/1.1 200 OK
//! ST: urn:smartspeaker-audio:service:SpeakerGroup:1
//! USN: uuid:RINCON_5CAAFDD347CA01400::urn:smartspeaker-audio:service:SpeakerGroup:1
//! GROUPINFO.SMARTSPEAKER.AUDIO: gc=1; gid=RINCON_5CAAFDD347CA01400:2367147317; gname="Clara's Bedroom"
//! WEBSOCK.SMARTSPEAKER.AUDIO: wss://10.10.237.28:1443/websocket/api
//! HOUSEHOLD.SMARTSPEAKER.AUDIO: Sonos_FVGVbNxG94Pbng2LLMm8zdSVuT.nErh-aF_Y_qPBGkAza3J
//! APIVER.SMARTSPEAKER.AUDIO: 1.54.1
//! ...
//! ```

use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    task::{Context, Poll, ready},
    time::Duration,
};

use futures_util::{Stream, StreamExt};
use tokio::{
    io::ReadBuf,
    net::UdpSocket,
    time::{Sleep, sleep},
};
use tracing::debug;
use url::Url;

use crate::{ConnectOptions, Connection, Error, GroupId, HouseholdId, PlayerId};

const SSDP_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900));
const SEARCH_TARGET: &str = "urn:smartspeaker-audio:service:SpeakerGroup:1";
/// UDP is unreliable, so send the search more than once.
const SEARCH_ATTEMPTS: usize = 2;

/// A player found by [`discover`] or [`discover_stream`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiscoveredPlayer {
    pub player_id: PlayerId,
    pub household_id: HouseholdId,
    pub websocket_url: Url,
    /// The address the player answered from.
    pub address: IpAddr,
    /// The group the player belongs to, if it said.
    pub group: Option<DiscoveredGroup>,
    pub api_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiscoveredGroup {
    pub id: GroupId,
    pub name: String,
    /// Whether the player is the coordinator of the group, i.e. the player to send the group's
    /// commands to.
    pub is_coordinator: bool,
}

impl DiscoveredPlayer {
    pub async fn connect(&self) -> Result<Connection, Error> {
        self.connect_with_options(ConnectOptions::default()).await
    }

    pub async fn connect_with_options(&self, options: ConnectOptions) -> Result<Connection, Error> {
        let options = options.household_id(self.household_id.clone());
        Connection::connect_to(&self.websocket_url, options).await
    }
}

/// Search the local network for players, collecting answers for `timeout`.
///
/// Players from all households on the network are returned; check
/// [`DiscoveredPlayer::household_id`] if that matters. Discovery uses IPv4 multicast on the
/// default interface, so it won't find players across VLANs or from inside most containers; use
/// [`Connection::connect`] with an address in that case.
///
/// Use [`discover_stream`] instead to use players as soon as they answer.
pub async fn discover(timeout: Duration) -> Result<Vec<DiscoveredPlayer>, Error> {
    Ok(discover_stream(timeout).await?.collect().await)
}

/// Search the local network for players, yielding them as they answer, until `timeout`.
///
/// Players usually answer within a few hundred milliseconds, so this allows connecting to one
/// without waiting for the whole timeout. The same caveats as for [`discover`] apply.
///
/// ```no_run
/// # use std::time::Duration;
/// use futures_util::StreamExt;
///
/// # async fn example() -> Result<(), sinuous_client::Error> {
/// let mut players = sinuous_client::discover_stream(Duration::from_secs(2)).await?;
/// if let Some(player) = players.next().await {
///     let conn = player.connect().await?;
/// }
/// # Ok(())
/// # }
/// ```
pub async fn discover_stream(timeout: Duration) -> Result<Discovery, Error> {
    Discovery::start(SSDP_ADDR, timeout).await
}

/// The players answering a search started by [`discover_stream`].
///
/// Each player is yielded once, even though players answer every search sent. The stream ends
/// once the timeout has elapsed.
#[derive(Debug)]
pub struct Discovery {
    socket: UdpSocket,
    deadline: Pin<Box<Sleep>>,
    seen: Vec<PlayerId>,
    buf: Box<[u8]>,
}

impl Discovery {
    async fn start(target: SocketAddr, timeout: Duration) -> Result<Self, Error> {
        let deadline = Box::pin(sleep(timeout));
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .await
            .map_err(|e| Error::Discovery(e.into()))?;
        let search = format!(
            "M-SEARCH * HTTP/1.1\r\n\
             HOST: {target}\r\n\
             MAN: \"ssdp:discover\"\r\n\
             MX: 1\r\n\
             ST: {SEARCH_TARGET}\r\n\
             \r\n"
        );
        for _ in 0..SEARCH_ATTEMPTS {
            socket
                .send_to(search.as_bytes(), target)
                .await
                .map_err(|e| Error::Discovery(e.into()))?;
        }
        Ok(Self {
            socket,
            deadline,
            seen: Vec::new(),
            buf: vec![0; 2048].into_boxed_slice(),
        })
    }
}

impl Stream for Discovery {
    type Item = DiscoveredPlayer;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.deadline.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            let mut buf = ReadBuf::new(&mut this.buf);
            let from = match ready!(this.socket.poll_recv_from(cx, &mut buf)) {
                Ok(from) => from,
                Err(e) => {
                    // e.g. ICMP port unreachable on some platforms; other answers may still come.
                    debug!("Error receiving SSDP response: {e}");
                    continue;
                }
            };
            let data = buf.filled();
            let Some(player) = parse_response(data, from.ip()) else {
                debug!(
                    "Ignoring SSDP response from {from}: {:?}",
                    String::from_utf8_lossy(data)
                );
                continue;
            };
            // Each player answers every search we sent.
            if this.seen.contains(&player.player_id) {
                continue;
            }
            debug!(
                "Discovered {} at {}",
                player.player_id, player.websocket_url
            );
            this.seen.push(player.player_id.clone());
            return Poll::Ready(Some(player));
        }
    }
}

fn parse_response(data: &[u8], address: IpAddr) -> Option<DiscoveredPlayer> {
    let text = std::str::from_utf8(data).ok()?;
    let mut lines = text.lines();
    if !lines.next()?.starts_with("HTTP/1.1 200") {
        return None;
    }
    let headers: Vec<(&str, &str)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim(), value.trim()))
        .collect();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, value)| *value)
    };

    if header("ST")? != SEARCH_TARGET {
        return None;
    }
    // `uuid:RINCON_5CAAFDD347CA01400::urn:smartspeaker-audio:service:SpeakerGroup:1`
    let player_id = header("USN")?.strip_prefix("uuid:")?.split("::").next()?;
    let household_id = header("HOUSEHOLD.SMARTSPEAKER.AUDIO")?;
    let websocket_url = Url::parse(header("WEBSOCK.SMARTSPEAKER.AUDIO")?).ok()?;

    Some(DiscoveredPlayer {
        player_id: PlayerId::new(player_id),
        household_id: HouseholdId::new(household_id),
        websocket_url,
        address,
        group: header("GROUPINFO.SMARTSPEAKER.AUDIO").and_then(parse_group_info),
        api_version: header("APIVER.SMARTSPEAKER.AUDIO").map(str::to_owned),
    })
}

/// Parse `gc=1; gid=RINCON_5CAAFDD347CA01400:2367147317; gname="Clara's Bedroom"`.
fn parse_group_info(value: &str) -> Option<DiscoveredGroup> {
    let (mut id, mut name, mut is_coordinator) = (None, None, false);
    let mut rest = value;
    while !rest.is_empty() {
        let (key, after_key) = rest.split_once('=')?;
        let (field, after_field) = match after_key.strip_prefix('"') {
            // Quoted values (group names) may contain `;`.
            Some(quoted) => {
                let (field, after) = quoted.split_once('"')?;
                (field, after.split_once(';').map_or("", |(_, rest)| rest))
            }
            None => after_key.split_once(';').unwrap_or((after_key, "")),
        };
        match key.trim() {
            "gc" => is_coordinator = field.trim() == "1",
            "gid" => id = Some(GroupId::new(field.trim())),
            "gname" => name = Some(field.to_owned()),
            _ => {}
        }
        rest = after_field.trim_start();
    }
    Some(DiscoveredGroup {
        id: id?,
        name: name.unwrap_or_default(),
        is_coordinator,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = "HTTP/1.1 200 OK\r\n\
        CACHE-CONTROL: max-age = 3600\r\n\
        EXT:\r\n\
        LOCATION: http://10.10.237.28:1400/xml/group_description.xml\r\n\
        SERVER: Linux UPnP/1.0 Sonos/97.1-80312 (ZPS17)\r\n\
        ST: urn:smartspeaker-audio:service:SpeakerGroup:1\r\n\
        USN: uuid:RINCON_5CAAFDD347CA01400::urn:smartspeaker-audio:service:SpeakerGroup:1\r\n\
        BOOTID.UPNP.ORG: 22\r\n\
        CONFIGID.UPNP.ORG: 1\r\n\
        GROUPINFO.SMARTSPEAKER.AUDIO: gc=1; gid=RINCON_5CAAFDD347CA01400:2367147317; gname=\"Clara's Bedroom\"\r\n\
        WEBSOCK.SMARTSPEAKER.AUDIO: wss://10.10.237.28:1443/websocket/api\r\n\
        HOUSEHOLD.SMARTSPEAKER.AUDIO: Sonos_FVGVbNxG94Pbng2LLMm8zdSVuT.nErh-aF_Y_qPBGkAza3J\r\n\
        LOCATION.SMARTSPEAKER.AUDIO: lc_10970ab61899420cb958c53886a5d257\r\n\
        APIVER.SMARTSPEAKER.AUDIO: 1.54.1\r\n\
        MINAPIVER.SMARTSPEAKER.AUDIO: 1.1.0\r\n\
        \r\n";

    fn address() -> IpAddr {
        "10.10.237.28".parse().unwrap()
    }

    #[test]
    fn parses_speaker_group_response() {
        let player = parse_response(RESPONSE.as_bytes(), address()).unwrap();
        assert_eq!(player.player_id, PlayerId::new("RINCON_5CAAFDD347CA01400"));
        assert_eq!(
            player.household_id,
            HouseholdId::new("Sonos_FVGVbNxG94Pbng2LLMm8zdSVuT.nErh-aF_Y_qPBGkAza3J")
        );
        assert_eq!(
            player.websocket_url.as_str(),
            "wss://10.10.237.28:1443/websocket/api"
        );
        assert_eq!(player.api_version.as_deref(), Some("1.54.1"));
        assert_eq!(
            player.group,
            Some(DiscoveredGroup {
                id: GroupId::new("RINCON_5CAAFDD347CA01400:2367147317"),
                name: "Clara's Bedroom".to_owned(),
                is_coordinator: true,
            })
        );
    }

    #[test]
    fn header_names_are_case_insensitive() {
        let response = RESPONSE
            .replace("WEBSOCK.SMARTSPEAKER.AUDIO", "websock.smartspeaker.audio")
            .replace("USN:", "Usn:");
        assert!(parse_response(response.as_bytes(), address()).is_some());
    }

    #[test]
    fn ignores_incomplete_or_unrelated_responses() {
        let without_websocket = RESPONSE.replace("WEBSOCK.SMARTSPEAKER.AUDIO", "X-OTHER");
        assert_eq!(
            parse_response(without_websocket.as_bytes(), address()),
            None
        );
        let other_target = RESPONSE.replace(
            "ST: urn:smartspeaker-audio:service:SpeakerGroup:1",
            "ST: upnp:rootdevice",
        );
        assert_eq!(parse_response(other_target.as_bytes(), address()), None);
        let not_ok = RESPONSE.replace("200 OK", "404 Not Found");
        assert_eq!(parse_response(not_ok.as_bytes(), address()), None);
        assert_eq!(parse_response(b"\xff\xfe", address()), None);
    }

    #[test]
    fn parses_group_info() {
        let group =
            parse_group_info(r#"gc=0; gid=RINCON_1:42; gname="Kitchen; Dining = Room""#).unwrap();
        assert_eq!(group.id, GroupId::new("RINCON_1:42"));
        assert_eq!(group.name, "Kitchen; Dining = Room");
        assert!(!group.is_coordinator);

        // Order and unknown keys don't matter.
        let group = parse_group_info(r#"gname="Office"; extra=1; gc=1; gid=RINCON_2:1"#).unwrap();
        assert_eq!(group.name, "Office");
        assert!(group.is_coordinator);

        assert_eq!(parse_group_info("gc=1; gname=\"No id\""), None);
    }

    /// Answer searches like a player would, with some garbage first. Returns where to send them.
    async fn fake_player() -> SocketAddr {
        let fake_player = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = fake_player.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0; 2048];
            loop {
                let (len, from) = fake_player.recv_from(&mut buf).await.unwrap();
                let search = std::str::from_utf8(&buf[..len]).unwrap();
                assert!(search.starts_with("M-SEARCH * HTTP/1.1\r\n"), "{search}");
                assert!(
                    search.contains(&format!("ST: {SEARCH_TARGET}\r\n")),
                    "{search}"
                );
                fake_player.send_to(b"garbage", from).await.unwrap();
                fake_player
                    .send_to(RESPONSE.as_bytes(), from)
                    .await
                    .unwrap();
            }
        });
        target
    }

    #[tokio::test]
    async fn collects_unique_players_until_timeout() {
        let target = fake_player().await;
        let players: Vec<_> = Discovery::start(target, Duration::from_millis(300))
            .await
            .unwrap()
            .collect()
            .await;
        // The player answered every attempt, but is only reported once.
        assert_eq!(players.len(), 1);
        assert_eq!(
            players[0].player_id,
            PlayerId::new("RINCON_5CAAFDD347CA01400")
        );
        assert_eq!(players[0].address, IpAddr::from(Ipv4Addr::LOCALHOST));
    }

    #[tokio::test]
    async fn yields_players_before_the_timeout() {
        let target = fake_player().await;
        let mut players = Discovery::start(target, Duration::from_secs(10))
            .await
            .unwrap();
        let player = tokio::time::timeout(Duration::from_secs(1), players.next())
            .await
            .expect("no player before the timeout")
            .unwrap();
        assert_eq!(player.player_id, PlayerId::new("RINCON_5CAAFDD347CA01400"));
    }
}
