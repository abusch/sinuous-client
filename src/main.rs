use std::sync::Arc;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use rustls::crypto::CryptoProvider;
use serde_json::{Value, json};
use tokio_tungstenite::{
    Connector, connect_async_tls_with_config,
    tungstenite::{ClientRequestBuilder, Message},
};

use crate::{
    conn::tls_config,
    model::{PrefixMessage, PrefixMessagePayload, SonosMsg, SonosObject},
};

mod conn;
mod model;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CryptoProvider::install_default(rustls::crypto::aws_lc_rs::default_provider()).unwrap();
    let req =
        // ClientRequestBuilder::new("wss://sonos-74CA6062DF36.local:1443/websocket/api".parse()?)
        ClientRequestBuilder::new("wss://10.0.1.52:1443/websocket/api".parse()?)
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

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let jh = tokio::spawn(async move {
        read.for_each(|message| async {
            match message {
                Ok(Message::Text(utf8_bytes)) => match decode_message(utf8_bytes.as_bytes()) {
                    Ok(sonos_msg) => {
                        if let Err(e) = tx.send(sonos_msg) {
                            eprintln!("Failed to send msg to channel: {e:#}");
                        }
                    }
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

    println!("Connection successful");

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
    write.send(Message::Text(json.to_string().into())).await?;

    let Some(SonosMsg(_prefix, SonosObject::Groups(groups))) = rx.recv().await else {
        bail!("Didn't receive any response to group::getGroups");
    };
    for g in &groups.groups {
        println!("Found group {} ({:?})", g.name, g.playback_state);
    }
    for p in &groups.players {
        println!("Found player {}", p.name);
        for d in &p.devices {
            println!("\tDevice {} ({})", d.name, d.model_display_name);
        }
    }

    let json = json!([
        {
            "namespace": "playbackMetadata",
            "command": "getMetadataStatus",
            "groupId": groups.groups[0].id.0.clone(),
            "sessionId": null,
            "cmdId": null
        },
        {
            "name": "Sonos Test",
            "appId": "com.test.sonos"
        }
    ]);
    write.send(Message::Text(json.to_string().into())).await?;

    let Some(SonosMsg(_prefix, SonosObject::MetadataStatus(meta))) = rx.recv().await else {
        bail!("Didn't receive any response to playbackMetadata::getMetadataStatus");
    };
    println!(
        "Playback metadata for group {}: {:#?}",
        groups.groups[0].name, meta
    );

    while let Some(SonosMsg(
        PrefixMessage {
            namespace,
            r#type: _,
            payload,
        },
        sonos_object,
    )) = rx.recv().await
    {
        match payload {
            PrefixMessagePayload::Reply { response, success } => {
                println!("Reply to {namespace}::{response} (success={success}): {sonos_object:#?}");
            }
            PrefixMessagePayload::Event { name } => {
                println!("Event {namespace}::{name}: {sonos_object:#?}");
            }
        }
    }

    jh.await?;
    Ok(())
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
