use anyhow::bail;
use rustls::crypto::CryptoProvider;

use crate::{model::SonosObject, sonos::Sonos};

mod conn;
mod model;
mod sonos;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CryptoProvider::install_default(rustls::crypto::aws_lc_rs::default_provider()).unwrap();
    let uri = "wss://10.0.1.52:1443/websocket/api".parse()?;
    let mut sonos = Sonos::connect(uri).await.unwrap();

    println!("Connection successful");

    let SonosObject::Groups(groups) = sonos.get_groups().await? else {
        bail!("Invalid response for get_groups");
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

    // let json = json!([
    //     {
    //         "namespace": "playbackMetadata",
    //         "command": "getMetadataStatus",
    //         "groupId": groups.groups[0].id.0.clone(),
    //         "sessionId": null,
    //         "cmdId": null
    //     },
    //     {
    //         "name": "Sonos Test",
    //         "appId": "com.test.sonos"
    //     }
    // ]);
    // write.send(Message::Text(json.to_string().into())).await?;
    //
    // let Some(SonosMsg(_prefix, SonosObject::MetadataStatus(meta))) = rx.recv().await else {
    //     bail!("Didn't receive any response to playbackMetadata::getMetadataStatus");
    // };
    // println!(
    //     "Playback metadata for group {}: {:#?}",
    //     groups.groups[0].name, meta
    // );
    //
    // while let Some(SonosMsg(
    //     PrefixMessage {
    //         namespace,
    //         r#type: _,
    //         payload,
    //     },
    //     sonos_object,
    // )) = rx.recv().await
    // {
    //     match payload {
    //         PrefixMessagePayload::Reply { response, success } => {
    //             println!("Reply to {namespace}::{response} (success={success}): {sonos_object:#?}");
    //         }
    //         PrefixMessagePayload::Event { name } => {
    //             println!("Event {namespace}::{name}: {sonos_object:#?}");
    //         }
    //     }
    // }

    sonos.shutdown().await;
    Ok(())
}
