//! Fake players for tests.

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::broadcast,
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::{
    Message,
    handshake::server::{Request, Response},
};
use url::Url;

use crate::connection::{DEFAULT_API_KEY, SUB_PROTOCOL};

/// A fake player accepting a single websocket connection.
pub(crate) struct FakePlayer {
    listener: TcpListener,
    pub addr: SocketAddr,
}

impl FakePlayer {
    pub async fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        Self { listener, addr }
    }

    pub fn url(&self) -> Url {
        url(self.addr)
    }

    /// Accept a single connection, answering each command with `handler(header, body)` (or not at
    /// all if it returns `None`), after an optional delay given by the command's `delay`
    /// parameter.
    pub fn serve(self, handler: impl Handler) -> JoinHandle<()> {
        tokio::spawn(async move {
            let (stream, _) = self.listener.accept().await.unwrap();
            // Nothing to push.
            let (_, push) = broadcast::channel(1);
            serve_connection(stream, Arc::new(handler), push).await;
        })
    }

    /// Like [`FakePlayer::serve`], but accept any number of connections, which can be dropped
    /// (and refused) to simulate network failures.
    pub fn serve_many(self, handler: impl Handler) -> FakeServer {
        let state = Arc::new(ServerState {
            available: AtomicBool::new(true),
            accepted: AtomicUsize::new(0),
            connections: Mutex::default(),
            push: broadcast::Sender::new(16),
        });
        let handler = Arc::new(handler);
        let server = FakeServer {
            state: state.clone(),
        };
        tokio::spawn(async move {
            loop {
                let (stream, _) = self.listener.accept().await.unwrap();
                state.accepted.fetch_add(1, Ordering::Relaxed);
                if !state.available.load(Ordering::Relaxed) {
                    continue;
                }
                let push = state.push.subscribe();
                let connection = tokio::spawn(serve_connection(stream, handler.clone(), push));
                state.connections.lock().unwrap().push(connection);
            }
        });
        server
    }
}

pub(crate) trait Handler:
    Fn(&Value, &Value) -> Option<Vec<Value>> + Send + Sync + 'static
{
}

impl<F: Fn(&Value, &Value) -> Option<Vec<Value>> + Send + Sync + 'static> Handler for F {}

/// Controls a fake player started with [`FakePlayer::serve_many`]. It keeps running when this is
/// dropped.
pub(crate) struct FakeServer {
    state: Arc<ServerState>,
}

struct ServerState {
    available: AtomicBool,
    accepted: AtomicUsize,
    connections: Mutex<Vec<JoinHandle<()>>>,
    /// Messages to send on all open connections.
    push: broadcast::Sender<Value>,
}

impl FakeServer {
    /// Drop the open connections, without closing them properly.
    pub fn drop_connections(&self) {
        for connection in self.state.connections.lock().unwrap().drain(..) {
            connection.abort();
        }
    }

    /// When unavailable, connections are accepted but dropped right away.
    pub fn set_available(&self, available: bool) {
        self.state.available.store(available, Ordering::Relaxed);
    }

    /// Send a message (e.g. an event) on all open connections.
    pub fn push(&self, message: Value) {
        self.state.push.send(message).unwrap();
    }

    /// The number of connections accepted so far, including the ones dropped while unavailable.
    pub fn accepted(&self) -> usize {
        self.state.accepted.load(Ordering::Relaxed)
    }
}

#[allow(clippy::result_large_err)] // The handshake callback's signature is imposed by tungstenite.
async fn serve_connection(
    stream: TcpStream,
    handler: Arc<impl Handler>,
    mut push: broadcast::Receiver<Value>,
) {
    let ws = tokio_tungstenite::accept_hdr_async(stream, |req: &Request, mut resp: Response| {
        assert_eq!(req.headers()["X-Sonos-Api-Key"], DEFAULT_API_KEY);
        resp.headers_mut()
            .insert("Sec-WebSocket-Protocol", SUB_PROTOCOL.parse().unwrap());
        Ok(resp)
    })
    .await
    .unwrap();
    let (write, mut read) = ws.split();
    let write = Arc::new(tokio::sync::Mutex::new(write));
    let mut pushing = true;
    loop {
        let msg = tokio::select! {
            msg = read.next() => msg,
            message = push.recv(), if pushing => {
                match message {
                    Ok(message) => {
                        let text = Message::text(message.to_string());
                        let _ = write.lock().await.send(text).await;
                    }
                    // Only stop once the server is gone: a lagging connection just misses some.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => pushing = false,
                }
                continue;
            }
        };
        let Some(Ok(msg)) = msg else { break };
        let Message::Text(text) = msg else { continue };
        let [header, body]: [Value; 2] = serde_json::from_str(text.as_str()).unwrap();
        let delay = body["delay"].as_u64().unwrap_or(0);
        let Some(replies) = handler(&header, &body) else {
            continue;
        };
        let write = write.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            for reply in replies {
                let text = Message::text(reply.to_string());
                // The connection may have been dropped in the meantime.
                if write.lock().await.send(text).await.is_err() {
                    break;
                }
            }
        });
    }
}

/// A player that accepts TCP connections but never completes the websocket handshake.
pub(crate) async fn stalled_player() -> (Url, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = url(listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    (url, server)
}

/// Start a fake player; see [`FakePlayer::serve`].
pub(crate) async fn fake_player(handler: impl Handler) -> (SocketAddr, JoinHandle<()>) {
    let player = FakePlayer::bind().await;
    let addr = player.addr;
    (addr, player.serve(handler))
}

pub(crate) fn url(addr: SocketAddr) -> Url {
    Url::parse(&format!("ws://{addr}/websocket/api")).unwrap()
}

/// A reply to the command with the given `header`.
pub(crate) fn reply(header: &Value, success: bool, body: Value) -> Vec<Value> {
    vec![json!([
        {"namespace": header["namespace"], "cmdId": header["cmdId"], "success": success},
        body
    ])]
}
