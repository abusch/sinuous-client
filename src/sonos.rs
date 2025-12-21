use std::sync::Arc;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt, lock::Mutex, stream::SplitSink};
use iddqd::IdHashMap;
use serde_json::{Value, json};
use tokio::{net::TcpStream, sync::mpsc::UnboundedReceiver};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
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
    write: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    jh: tokio::task::JoinHandle<()>,
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
        let (write, read) = ws_stream.split();

        let (events_tx, events_rx) = tokio::sync::mpsc::unbounded_channel();
        let (responses_tx, responses_rx) = tokio::sync::mpsc::unbounded_channel();

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
                    Ok(_) => {
                        eprintln!("Unsupported websocket message typte");
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
            write,
            jh,
        })
    }

    pub async fn get_groups(&mut self) -> anyhow::Result<SonosObject> {
        let json = json!([
            {
                "namespace": "groups",
                "command": "getGroups",
                "householdId": "Sonos_FVGVbNxG94Pbng2LLMm8zdSVuT.nErh-aF_Y_qPBGkAza3J",
                "sessionId": null,
                "cmdId": null
            },
            {
                "name": "Sonos Test",
                "appId": "com.test.sonos"
            }
        ]);
        self.write
            .send(Message::Text(json.to_string().into()))
            .await?;
        let resp = self.responses_rx.recv().await.unwrap();
        Ok(resp.unwrap())
    }

    pub async fn shutdown(self) {
        self.jh.await.unwrap();
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
