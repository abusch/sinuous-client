use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::{Serialize, de::DeserializeOwned, de::IgnoredAny};
use serde_json::Value;
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc, oneshot},
    task::JoinHandle,
};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
    tungstenite::{ClientRequestBuilder, Message, http::Uri},
};
use tracing::{debug, error, info, trace, warn};
use url::Url;

use crate::{
    Error, GroupId, HouseholdId, PlayerId,
    events::Event,
    protocol::{self, Incoming, NoParams, ProtocolError, RequestHeader, Target},
    tls::tls_config,
};

/// The API key used by default. Sonos players accept this well-known key for local control.
pub const DEFAULT_API_KEY: &str = "12345678-abcd-1234-5678-123456789000";
pub(crate) const SUB_PROTOCOL: &str = "v1.api.smartspeaker.audio";
const EVENT_CHANNEL_CAPACITY: usize = 256;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// `cmdId` of the request used to learn the household ID. Regular commands start at 1.
const HOUSEHOLD_CMD_ID: &str = "0";

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Responder = oneshot::Sender<Result<Value, Error>>;

/// Options used when connecting to a player.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConnectOptions {
    /// Value of the `X-Sonos-Api-Key` header.
    pub api_key: String,
    /// How long to wait for a reply to a command.
    pub request_timeout: Duration,
    /// The household the player belongs to, if already known. Otherwise it is asked for when
    /// connecting, which costs a round-trip.
    pub household_id: Option<HouseholdId>,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            api_key: DEFAULT_API_KEY.to_owned(),
            request_timeout: Duration::from_secs(10),
            household_id: None,
        }
    }
}

impl ConnectOptions {
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = api_key.into();
        self
    }

    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn household_id(mut self, household_id: HouseholdId) -> Self {
        self.household_id = Some(household_id);
        self
    }
}

/// A websocket connection to a single Sonos player.
///
/// Household-scoped commands (groups, favorites, playlists...) can be sent to any player in the
/// household, but the other commands only work on specific players:
///
/// * group commands must be sent to the group's coordinator; other players reply with
///   [`Error::GroupCoordinatorChanged`].
/// * player commands must be sent to the player itself; other players reply with an
///   `ERROR_INVALID_OBJECT_ID` [`Error::Api`].
///
/// [`Household`](crate::Household) takes care of this.
///
/// `Connection` is cheap to clone, and commands can be issued concurrently from several tasks.
/// The underlying websocket is closed when the last clone is dropped, or explicitly with
/// [`Connection::close`].
#[derive(Debug, Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    household_id: HouseholdId,
    websocket_url: Url,
    request_timeout: Duration,
    next_cmd_id: AtomicU64,
    pending: Arc<Pending>,
    write_tx: mpsc::UnboundedSender<Message>,
    events_tx: broadcast::Sender<Event>,
    reader_task: Mutex<Option<JoinHandle<()>>>,
    writer_task: JoinHandle<()>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.writer_task.abort();
        if let Some(reader) = lock(&self.reader_task).take() {
            reader.abort();
        }
    }
}

impl Connection {
    /// Connect to the player at `host` (a hostname or IP address) with default options.
    pub async fn connect(host: &str) -> Result<Self, Error> {
        Self::connect_with_options(host, ConnectOptions::default()).await
    }

    /// Connect to the player at `host` (a hostname or IP address).
    pub async fn connect_with_options(host: &str, options: ConnectOptions) -> Result<Self, Error> {
        let mut websocket_url =
            Url::parse("wss://localhost:1443/websocket/api").expect("valid static URL");
        websocket_url
            .set_host(Some(host))
            .map_err(|e| Error::Connect(Box::new(e)))?;
        Self::connect_to(&websocket_url, options).await
    }

    /// Connect to a known websocket URL, e.g. [`Player::websocket_url`](crate::groups::Player)
    /// or [`GroupCoordinatorChanged::websocket_url`](crate::groups::GroupCoordinatorChanged).
    pub async fn connect_to(websocket_url: &Url, options: ConnectOptions) -> Result<Self, Error> {
        info!("Connecting to {websocket_url}...");
        let uri: Uri = websocket_url
            .as_str()
            .parse()
            .map_err(|e| Error::Connect(Box::new(e)))?;
        let request = ClientRequestBuilder::new(uri)
            .with_header("X-Sonos-Api-Key", options.api_key.clone())
            .with_sub_protocol(SUB_PROTOCOL);
        let tls = tls_config().map_err(Error::Connect)?;
        let (mut ws, _) = connect_async_tls_with_config(
            request,
            None,
            false,
            Some(Connector::Rustls(Arc::new(tls))),
        )
        .await
        .map_err(|e| Error::Connect(Box::new(e)))?;

        let household_id = match options.household_id {
            Some(household_id) => household_id,
            None => tokio::time::timeout(options.request_timeout, fetch_household_id(&mut ws))
                .await
                .map_err(|_| Error::Timeout)??,
        };
        debug!("Connected to household {household_id}");

        Ok(Self::spawn(
            ws,
            websocket_url.clone(),
            household_id,
            options.request_timeout,
        ))
    }

    fn spawn(
        ws: WsStream,
        websocket_url: Url,
        household_id: HouseholdId,
        request_timeout: Duration,
    ) -> Self {
        let (write, read) = ws.split();
        let (write_tx, write_rx) = mpsc::unbounded_channel();
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let pending = Arc::new(Pending::new());

        let writer_task = tokio::spawn(write_loop(write, write_rx));
        let reader_task = tokio::spawn(read_loop(
            read,
            pending.clone(),
            events_tx.clone(),
            write_tx.clone(),
        ));

        Self {
            inner: Arc::new(Inner {
                household_id,
                websocket_url,
                request_timeout,
                next_cmd_id: AtomicU64::new(1),
                pending,
                write_tx,
                events_tx,
                reader_task: Mutex::new(Some(reader_task)),
                writer_task,
            }),
        }
    }

    pub fn household_id(&self) -> &HouseholdId {
        &self.inner.household_id
    }

    pub fn websocket_url(&self) -> &Url {
        &self.inner.websocket_url
    }

    /// Whether the websocket has been closed, by either side.
    pub fn is_closed(&self) -> bool {
        lock(&self.inner.pending.0).is_none()
    }

    /// Commands targeting the given group.
    pub fn group(&self, id: &GroupId) -> GroupHandle {
        GroupHandle {
            conn: self.clone(),
            id: id.clone(),
        }
    }

    /// Commands targeting the given player.
    pub fn player(&self, id: &PlayerId) -> PlayerHandle {
        PlayerHandle {
            conn: self.clone(),
            id: id.clone(),
        }
    }

    /// Receive events for the namespaces this connection is subscribed to.
    ///
    /// Each receiver gets every event sent after it was created, so create it *before* calling
    /// [`Connection::subscribe`] to see the initial event a player sends on subscription.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.inner.events_tx.subscribe()
    }

    /// Close the websocket and wait for the player to acknowledge.
    ///
    /// Pending and future commands on this connection (and its clones) fail with
    /// [`Error::ConnectionClosed`].
    pub async fn close(&self) {
        let _ = self.inner.write_tx.send(Message::Close(None));
        let reader = lock(&self.inner.reader_task).take();
        if let Some(reader) = reader
            && tokio::time::timeout(CLOSE_TIMEOUT, reader).await.is_err()
        {
            warn!("Timed out waiting for the connection to close");
        }
    }

    pub(crate) fn household_target(&self) -> Target<'_> {
        Target::Household(&self.inner.household_id)
    }

    /// Send a command and decode its response as `T`.
    pub(crate) async fn request<T: DeserializeOwned>(
        &self,
        namespace: &'static str,
        command: &'static str,
        target: Target<'_>,
        params: &impl Serialize,
    ) -> Result<T, Error> {
        let cmd_id = self
            .inner
            .next_cmd_id
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        let header = RequestHeader {
            namespace,
            command,
            cmd_id: &cmd_id,
            target: Some(target),
        };
        let text = protocol::encode_request(&header, params)
            .map_err(|source| ProtocolError::Body { command, source })?;

        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(cmd_id.clone(), tx)?;
        // Make sure the pending entry is cleaned up on timeout or if this future is dropped.
        let _guard = PendingGuard {
            pending: &self.inner.pending,
            cmd_id: &cmd_id,
        };

        debug!(namespace, command, cmd_id, "Sending command");
        trace!("Sending {text}");
        self.inner
            .write_tx
            .send(Message::text(text))
            .map_err(|_| Error::ConnectionClosed)?;

        let body = tokio::time::timeout(self.inner.request_timeout, rx)
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::ConnectionClosed)??;

        T::deserialize(&body).map_err(|source| ProtocolError::Body { command, source }.into())
    }

    /// Send a command whose response carries no data.
    pub(crate) async fn command(
        &self,
        namespace: &'static str,
        command: &'static str,
        target: Target<'_>,
        params: &impl Serialize,
    ) -> Result<(), Error> {
        self.request::<IgnoredAny>(namespace, command, target, params)
            .await
            .map(|_| ())
    }
}

/// Learn which household the player belongs to.
///
/// There is no command for this (`getHouseholds` isn't supported locally), but players include
/// `householdId` in the header of every reply, including errors. So send a household command
/// without a household and look at the header of the (failed) reply.
async fn fetch_household_id(ws: &mut WsStream) -> Result<HouseholdId, Error> {
    let header = RequestHeader {
        namespace: "groups",
        command: "getGroups",
        cmd_id: HOUSEHOLD_CMD_ID,
        target: None,
    };
    let text =
        protocol::encode_request(&header, &NoParams {}).map_err(|source| ProtocolError::Body {
            command: header.command,
            source,
        })?;
    ws.send(Message::text(text))
        .await
        .map_err(|e| Error::Connect(Box::new(e)))?;

    while let Some(msg) = ws.next().await {
        let msg = msg.map_err(|e| Error::Connect(Box::new(e)))?;
        let Message::Text(text) = msg else { continue };
        if let Incoming::Reply {
            cmd_id: Some(cmd_id),
            household_id,
            ..
        } = protocol::decode(text.as_str())?
            && cmd_id == HOUSEHOLD_CMD_ID
        {
            return household_id.ok_or_else(|| ProtocolError::MissingHousehold.into());
        }
    }
    Err(Error::ConnectionClosed)
}

/// Commands targeting a single group. Obtained with [`Connection::group`].
#[derive(Debug, Clone)]
pub struct GroupHandle {
    pub(crate) conn: Connection,
    pub(crate) id: GroupId,
}

impl GroupHandle {
    pub fn id(&self) -> &GroupId {
        &self.id
    }

    pub(crate) fn target(&self) -> Target<'_> {
        Target::Group(&self.id)
    }
}

/// Commands targeting a single player. Obtained with [`Connection::player`].
#[derive(Debug, Clone)]
pub struct PlayerHandle {
    pub(crate) conn: Connection,
    pub(crate) id: PlayerId,
}

impl PlayerHandle {
    pub fn id(&self) -> &PlayerId {
        &self.id
    }

    pub(crate) fn target(&self) -> Target<'_> {
        Target::Player(&self.id)
    }
}

/// Commands awaiting a reply, keyed by `cmdId`. `None` once the connection is closed.
#[derive(Debug)]
struct Pending(Mutex<Option<HashMap<String, Responder>>>);

impl Pending {
    fn new() -> Self {
        Self(Mutex::new(Some(HashMap::new())))
    }

    fn insert(&self, cmd_id: String, responder: Responder) -> Result<(), Error> {
        match lock(&self.0).as_mut() {
            Some(map) => {
                map.insert(cmd_id, responder);
                Ok(())
            }
            None => Err(Error::ConnectionClosed),
        }
    }

    fn take(&self, cmd_id: &str) -> Option<Responder> {
        lock(&self.0).as_mut()?.remove(cmd_id)
    }

    /// Mark the connection as closed. Dropping the responders wakes up all waiting commands.
    fn close(&self) {
        lock(&self.0).take();
    }
}

struct PendingGuard<'a> {
    pending: &'a Pending,
    cmd_id: &'a str,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.pending.take(self.cmd_id);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn write_loop(
    mut write: SplitSink<WsStream, Message>,
    mut write_rx: mpsc::UnboundedReceiver<Message>,
) {
    while let Some(msg) = write_rx.recv().await {
        if let Err(e) = write.send(msg).await {
            error!("Failed to send message: {e}");
            break;
        }
    }
}

async fn read_loop(
    mut read: SplitStream<WsStream>,
    pending: Arc<Pending>,
    events_tx: broadcast::Sender<Event>,
    write_tx: mpsc::UnboundedSender<Message>,
) {
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => dispatch(text.as_str(), &pending, &events_tx),
            Ok(Message::Ping(payload)) => {
                trace!("Got ping, sending pong");
                let _ = write_tx.send(Message::Pong(payload));
            }
            Ok(Message::Close(frame)) => debug!("Received close frame: {frame:?}"),
            Ok(msg) => warn!("Unsupported websocket message type: {msg:?}"),
            Err(e) => {
                error!("Failed to read message: {e}");
                break;
            }
        }
    }
    debug!("Connection closed");
    pending.close();
}

fn dispatch(text: &str, pending: &Pending, events_tx: &broadcast::Sender<Event>) {
    trace!("Received {text}");
    match protocol::decode(text) {
        Ok(Incoming::Reply {
            cmd_id: Some(cmd_id),
            result,
            ..
        }) => match pending.take(&cmd_id) {
            // The receiver may have given up (timeout or cancellation), which is fine.
            Some(responder) => {
                let _ = responder.send(result);
            }
            None => debug!("Dropping reply to unknown or abandoned command {cmd_id}"),
        },
        Ok(Incoming::Reply { cmd_id: None, .. }) => warn!("Dropping reply without a cmdId"),
        Ok(Incoming::Event(event)) => {
            debug!(
                namespace = event.namespace,
                name = event.name,
                "Received event"
            );
            // Fails only if nobody is listening.
            let _ = events_tx.send(*event);
        }
        Err(e) => error!("Failed to decode message: {e}: {text}"),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use serde_json::json;

    use super::*;
    use crate::{
        events::{EventPayload, Subscription},
        test_support::{fake_player, reply, url},
    };

    async fn connect(addr: SocketAddr) -> Connection {
        let options = ConnectOptions::default()
            .request_timeout(Duration::from_millis(500))
            .household_id(HouseholdId::new("Sonos_1"));
        Connection::connect_to(&url(addr), options).await.unwrap()
    }

    #[tokio::test]
    async fn learns_household_from_reply_header() {
        let (addr, _) = fake_player(|header, _| {
            // Only the household lookup is expected: a command without any target.
            let ok = header["namespace"] == "groups"
                && header["command"] == "getGroups"
                && header.get("householdId").is_none()
                && header.get("groupId").is_none()
                && header.get("playerId").is_none();
            assert!(ok, "unexpected command {header}");
            Some(vec![json!([
                {"namespace": "groups", "householdId": "Sonos_42", "response": "getGroups",
                 "success": false, "type": "globalError", "cmdId": header["cmdId"]},
                {"_objectType": "globalError", "errorCode": "ERROR_MISSING_PARAMETERS",
                 "reason": "Missing householdId"}
            ])])
        })
        .await;
        let conn = Connection::connect_to(&url(addr), ConnectOptions::default())
            .await
            .unwrap();
        assert_eq!(conn.household_id(), &HouseholdId::new("Sonos_42"));
    }

    #[tokio::test]
    async fn fails_to_connect_without_household() {
        let (addr, _) = fake_player(|header, _| Some(reply(header, false, json!({})))).await;
        let err = Connection::connect_to(&url(addr), ConnectOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Protocol(_)), "{err:?}");
    }

    #[tokio::test]
    async fn correlates_out_of_order_replies() {
        let (addr, _) =
            fake_player(|header, body| Some(reply(header, true, json!({"echo": body["n"]})))).await;
        let conn = connect(addr).await;

        #[derive(serde::Deserialize)]
        struct Echo {
            echo: u64,
        }
        let target = conn.household_target();
        // The first command is answered last.
        let params = [
            json!({"n": 1, "delay": 200}),
            json!({"n": 2, "delay": 100}),
            json!({"n": 3}),
        ];
        let (a, b, c) = tokio::join!(
            conn.request::<Echo>("test", "a", target, &params[0]),
            conn.request::<Echo>("test", "b", target, &params[1]),
            conn.request::<Echo>("test", "c", target, &params[2]),
        );
        assert_eq!(a.unwrap().echo, 1);
        assert_eq!(b.unwrap().echo, 2);
        assert_eq!(c.unwrap().echo, 3);
        assert!(lock(&conn.inner.pending.0).as_ref().unwrap().is_empty());
    }

    #[tokio::test]
    async fn sends_target_and_params() {
        let (addr, _) = fake_player(|header, body| {
            let ok = header["namespace"] == "groupVolume"
                && header["command"] == "setVolume"
                && header["groupId"] == "RINCON_1:42"
                && *body == json!({"volume": 20});
            Some(reply(header, ok, json!({})))
        })
        .await;
        let conn = connect(addr).await;
        conn.group(&GroupId::new("RINCON_1:42"))
            .set_volume(20)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reports_api_errors() {
        let (addr, _) = fake_player(|header, _| {
            Some(reply(
                header,
                false,
                json!({"_objectType": "globalError", "errorCode": "ERROR_INVALID_PARAMETER", "reason": "nope"}),
            ))
        })
        .await;
        let conn = connect(addr).await;
        let err = conn.get_groups().await.unwrap_err();
        let Error::Api(api) = err else {
            panic!("unexpected error {err:?}");
        };
        assert_eq!(api.error_code, "ERROR_INVALID_PARAMETER");
    }

    #[tokio::test]
    async fn times_out_and_cleans_up() {
        let (addr, _) = fake_player(|_, _| None).await;
        let conn = connect(addr).await;
        let err = conn.get_groups().await.unwrap_err();
        assert!(matches!(err, Error::Timeout), "{err:?}");
        assert!(lock(&conn.inner.pending.0).as_ref().unwrap().is_empty());
    }

    #[tokio::test]
    async fn fails_pending_commands_when_connection_drops() {
        let (addr, server) = fake_player(|_, _| None).await;
        let conn = connect(addr).await;
        let request = conn.get_groups();
        let abort = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            server.abort();
        };
        let (result, ()) = tokio::join!(request, abort);
        assert!(matches!(result, Err(Error::ConnectionClosed)), "{result:?}");
        // Later commands fail immediately.
        let result = conn.get_groups().await;
        assert!(matches!(result, Err(Error::ConnectionClosed)), "{result:?}");
    }

    #[tokio::test]
    async fn delivers_events_after_subscribing() {
        let (addr, _) = fake_player(|header, _| {
            let mut replies = reply(header, true, json!({}));
            replies.push(json!([
                {"namespace": "groupVolume", "name": "groupVolume", "groupId": header["groupId"]},
                {"_objectType": "groupVolume", "volume": 30, "muted": true, "fixed": false}
            ]));
            Some(replies)
        })
        .await;
        let conn = connect(addr).await;
        let mut events = conn.events();
        let group = GroupId::new("RINCON_1:42");
        conn.subscribe(&Subscription::GroupVolume(group.clone()))
            .await
            .unwrap();

        let event = events.recv().await.unwrap();
        assert_eq!(event.group_id, Some(group));
        let EventPayload::GroupVolume(volume) = event.payload else {
            panic!("unexpected payload {:?}", event.payload);
        };
        assert_eq!(volume.volume, 30);
        assert!(volume.muted);
    }
}
