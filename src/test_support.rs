//! Fake players for tests.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};
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

    /// Answer each command with `handler(header, body)` (or not at all if it returns `None`),
    /// after an optional delay given by the command's `delay` parameter.
    #[allow(clippy::result_large_err)] // The handshake callback's signature is imposed by tungstenite.
    pub fn serve(
        self,
        handler: impl Fn(&Value, &Value) -> Option<Vec<Value>> + Send + Sync + 'static,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let (stream, _) = self.listener.accept().await.unwrap();
            let ws =
                tokio_tungstenite::accept_hdr_async(stream, |req: &Request, mut resp: Response| {
                    assert_eq!(req.headers()["X-Sonos-Api-Key"], DEFAULT_API_KEY);
                    resp.headers_mut()
                        .insert("Sec-WebSocket-Protocol", SUB_PROTOCOL.parse().unwrap());
                    Ok(resp)
                })
                .await
                .unwrap();
            let (write, mut read) = ws.split();
            let write = Arc::new(tokio::sync::Mutex::new(write));
            while let Some(Ok(msg)) = read.next().await {
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
                        write.lock().await.send(text).await.unwrap();
                    }
                });
            }
        })
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
pub(crate) async fn fake_player(
    handler: impl Fn(&Value, &Value) -> Option<Vec<Value>> + Send + Sync + 'static,
) -> (SocketAddr, JoinHandle<()>) {
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
