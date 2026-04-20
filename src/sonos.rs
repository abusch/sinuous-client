use std::sync::Arc;

use futures_util::{SinkExt, StreamExt, lock::Mutex};
use iddqd::IdHashMap;
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{ClientRequestBuilder, Message, http::Uri},
};
use tracing::{debug, error, info, warn};

use crate::{
    Error,
    conn::tls_config,
    model::{Group, GroupId, Player, PrefixMessage, PrefixMessagePayload, SonosMsg, SonosObject},
};

#[derive(Debug, Default)]
pub struct Topology {
    pub groups: IdHashMap<Group>,
    pub players: IdHashMap<Player>,
}

#[derive(Debug)]
pub struct Sonos {
    pub topology: Arc<Mutex<Topology>>,
    pub events_rx: UnboundedReceiver<SonosObject>,
    pub responses_rx: UnboundedReceiver<Result<SonosObject, Error>>,
    pub write_tx: UnboundedSender<Message>,
    writer_task: tokio::task::JoinHandle<()>,
    reader_task: tokio::task::JoinHandle<()>,
}

impl Sonos {
    pub async fn connect(uri: Uri) -> Result<Self, Error> {
        info!("Connecting to {uri}...");
        let req = ClientRequestBuilder::new(uri)
            .with_header("X-Sonos-Api-Key", "12345678-abcd-1234-5678-123456789000")
            .with_sub_protocol("v1.api.smartspeaker.audio");

        let tls_config = tls_config()?;
        let (ws_stream, _) = connect_async_tls_with_config(
            req,
            None,
            false,
            Some(Connector::Rustls(Arc::new(tls_config))),
        )
        .await?;
        let (mut write, read) = ws_stream.split();

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (responses_tx, responses_rx) = mpsc::unbounded_channel();
        let (write_tx, mut write_rx) = mpsc::unbounded_channel();

        // writer task
        let writer_task = tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                debug!("Sending message");
                if let Err(e) = write.send(msg).await {
                    error!("Failed to send message: {e}");
                }
            }
        });

        // reader task
        let write_tx2 = write_tx.clone();
        let jh = tokio::spawn(async move {
            read.for_each(|message| async {
                match message {
                    Ok(Message::Text(utf8_bytes)) => match decode_message(utf8_bytes.as_bytes()) {
                        Ok(SonosMsg(PrefixMessage { payload, .. }, object)) => match payload {
                            PrefixMessagePayload::Event { name: _ } => {
                                if let Err(e) = events_tx.send(object) {
                                    error!("Failed to send event to channel: {e}");
                                }
                            }
                            PrefixMessagePayload::Reply {
                                response: _,
                                success,
                            } => {
                                let res = success
                                    .then_some(object.clone())
                                    .ok_or(Error::ApiResponse(Box::new(object)));
                                if let Err(e) = responses_tx.send(res) {
                                    error!("Failed to send response to channel: {e}");
                                }
                            }
                        },
                        Err(e) => {
                            error!("Failed to decode text message: {e:?}");
                        }
                    },
                    Ok(Message::Ping(payload)) => {
                        debug!("Got ping, sending pong");
                        write_tx2
                            .send(Message::Pong(payload))
                            .expect("Failed to send pong");
                    }
                    Ok(Message::Close(_)) => {
                        info!("Connection is closing");
                    }
                    Ok(msg) => {
                        warn!("Unsupported websocket message type: {:?}", msg);
                    }
                    Err(e) => {
                        error!("Failed to read message: {e}");
                    }
                }
            })
            .await;
        });

        Ok(Self {
            topology: Arc::new(Mutex::new(Topology::default())),
            events_rx,
            responses_rx,
            write_tx,
            writer_task,
            reader_task: jh,
        })
    }

    pub async fn get_groups(&mut self, household_id: &str) -> Result<SonosObject, Error> {
        let json = json!([
            {
                "namespace": "groups",
                "command": "getGroups",
                "householdId": household_id,
            },
            {
                "name": "Sonos Test",
                "appId": "com.test.sonos"
            }
        ]);
        self.write_tx.send(Message::Text(json.to_string().into()))?;

        self.responses_rx
            .recv()
            .await
            .ok_or(Error::ConnectionClosed)?
    }

    pub async fn get_playback_status(&mut self, group_id: &GroupId) -> Result<SonosObject, Error> {
        let json = json!([
            {
                "namespace": "playback",
                "command": "getPlaybackStatus",
                "groupId": group_id,
            },
            {
                "name": "Sonos Test",
                "appId": "com.test.sonos"
            }
        ]);
        self.write_tx.send(Message::Text(json.to_string().into()))?;

        self.responses_rx
            .recv()
            .await
            .ok_or(Error::ConnectionClosed)?
    }

    pub async fn get_metadata_status(&mut self, group_id: &GroupId) -> Result<SonosObject, Error> {
        let json = json!([
            {
                "namespace": "playbackMetadata",
                "command": "getMetadataStatus",
                "groupId": group_id,
            },
            {
                "name": "Sonos Test",
                "appId": "com.test.sonos"
            }
        ]);
        self.write_tx.send(Message::Text(json.to_string().into()))?;

        self.responses_rx
            .recv()
            .await
            .ok_or(Error::ConnectionClosed)?
    }

    pub async fn shutdown(self) {
        self.write_tx.send(Message::Close(None)).unwrap();
        self.reader_task.await.unwrap();
    }
}

fn decode_message(payload: &[u8]) -> Result<SonosMsg, Error> {
    let objects = serde_json::from_slice::<Vec<Value>>(payload)?;

    let Ok([prefix, msg]) = <[Value; 2]>::try_from(objects) else {
        return Err(Error::InvalidResponse(
            "Expected an array with 2 elements".to_string(),
        ));
    };

    let prefix_msg = serde_json::from_value::<PrefixMessage>(prefix)?;
    debug!("prefix: {:?}", prefix_msg);
    debug!("message: {msg}");
    let sonos_object = serde_json::from_value::<SonosObject>(msg)?;

    Ok(SonosMsg(prefix_msg, sonos_object))
}
