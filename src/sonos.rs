use std::sync::Arc;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt, lock::Mutex};
use iddqd::IdHashMap;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{ClientRequestBuilder, Message, http::Uri},
};

use crate::{
    conn::tls_config,
    model::{Group, Player, PrefixMessage, PrefixMessagePayload, SonosMsg, SonosObject},
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
    pub responses_rx: UnboundedReceiver<Result<SonosObject, SonosObject>>,
    pub write_tx: UnboundedSender<Message>,
    writer_task: tokio::task::JoinHandle<()>,
    reader_task: tokio::task::JoinHandle<()>,
}

impl Sonos {
    pub async fn connect(uri: Uri) -> anyhow::Result<Self> {
        let req = ClientRequestBuilder::new(uri)
            .with_header("X-Sonos-Api-Key", "12345678-abcd-1234-5678-123456789000")
            .with_sub_protocol("v1.api.smartspeaker.audio");

        let tls_config = tls_config()?;
        println!("Connecting...");
        let (ws_stream, _) = connect_async_tls_with_config(
            req,
            None,
            false,
            Some(Connector::Rustls(Arc::new(tls_config))),
        )
        .await?;
        let (mut write, read) = ws_stream.split();

        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        let (responses_tx, responses_rx) = tokio::sync::mpsc::unbounded_channel();
        let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel();

        // writer task
        let writer_task = tokio::spawn(async move {
            while let Some(msg) = write_rx.recv().await {
                println!("Sending message");
                write.send(msg).await.expect("Failed to send message");
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
                                    eprintln!("Failed to send event to channel: {e}");
                                }
                            }
                            PrefixMessagePayload::Reply {
                                response: _,
                                success,
                            } => {
                                let res = success.then_some(object.clone()).ok_or(object);
                                if let Err(e) = responses_tx.send(res) {
                                    eprintln!("Failed to send response to channel: {e}");
                                }
                            }
                        },
                        Err(e) => {
                            println!("Failed to decode text message: {e:?}");
                        }
                    },
                    Ok(Message::Ping(payload)) => {
                        println!("Got ping, sending pong");
                        write_tx2
                            .send(Message::Pong(payload))
                            .expect("Failed to send pong");
                    }
                    Ok(Message::Close(_)) => {
                        println!("Connection is closing");
                    }
                    Ok(msg) => {
                        eprintln!("Unsupported websocket message type: {:?}", msg);
                    }
                    Err(e) => {
                        eprintln!("Failed to read message: {e}");
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

    pub async fn get_groups(&mut self) -> anyhow::Result<SonosObject> {
        let json = json!([
            {
                "namespace": "groups",
                "command": "getGroups",
                "householdId": "Sonos_FVGVbNxG94Pbng2LLMm8zdSVuT.nErh-aF_Y_qPBGkAza3J",
            },
            {
                "name": "Sonos Test",
                "appId": "com.test.sonos"
            }
        ]);
        self.write_tx.send(Message::Text(json.to_string().into()))?;
        let resp = self.responses_rx.recv().await.unwrap();
        Ok(resp.unwrap())
    }

    pub async fn shutdown(self) {
        self.write_tx.send(Message::Close(None)).unwrap();
        self.reader_task.await.unwrap();
    }
}

pub fn decode_message(payload: &[u8]) -> anyhow::Result<SonosMsg> {
    let objects = serde_json::from_slice::<Vec<Value>>(payload).context("Expected an array")?;

    let Ok([prefix, msg]) = <[Value; 2]>::try_from(objects) else {
        bail!("Expected an array with 2 elements");
    };

    let prefix_msg = serde_json::from_value::<PrefixMessage>(prefix)?;
    println!("{:?}", prefix_msg);
    println!("{msg}");
    let sonos_object =
        serde_json::from_value::<SonosObject>(msg).context("Object was not a sonos object...")?;

    Ok(SonosMsg(prefix_msg, sonos_object))
}
