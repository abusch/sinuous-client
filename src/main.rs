use anyhow::bail;
use iddqd::IdHashMap;
use rustls::crypto::CryptoProvider;

use sonos_ws::{model::SonosObject, sonos::Sonos};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("sonos_ws=debug"))
        .init();
    CryptoProvider::install_default(rustls::crypto::aws_lc_rs::default_provider()).unwrap();
    let uri = "wss://10.10.190.82:1443/websocket/api".parse()?;
    let mut sonos = Sonos::connect(uri).await.unwrap();

    println!("Connection successful");

    let household_id = "Sonos_FVGVbNxG94Pbng2LLMm8zdSVuT.nErh-aF_Y_qPBGkAza3J";
    let SonosObject::Groups(groups) = sonos.get_groups(household_id).await? else {
        bail!("Invalid response for get_groups");
    };

    let mut group_map = IdHashMap::new();
    let mut player_map = IdHashMap::new();
    for g in groups.groups {
        group_map.insert_unique(g).expect("Duplicate group!");
    }
    for p in groups.players {
        player_map.insert_unique(p).expect("Duplicate player!");
    }

    for g in &group_map {
        let coordinator = &g.coordinator_id;
        let Some(coord_player) = player_map.get(coordinator) else {
            println!("Missing coordinator!");
            continue;
        };
        println!(
            "Found group {} ({:?}), coordinator = {}",
            g.name, g.playback_state, coord_player.name
        );
    }
    for p in &player_map {
        println!("Found player {}", p.name);
        for d in &p.devices {
            println!("\tDevice {} ({})", d.name, d.model_display_name);
        }
    }

    let group = &group_map.iter().next().unwrap();

    let SonosObject::PlaybackStatus(status) = sonos.get_playback_status(&group.id).await? else {
        bail!("Invalid response for get_playback_status");
    };
    println!("Playback status for group {}: {:#?}", group.name, status);

    let SonosObject::MetadataStatus(status) = sonos.get_metadata_status(&group.id).await? else {
        bail!("Invalid response for get_metadata_status");
    };
    println!("Playback metadata for group {}: {:#?}", group.name, status);

    sonos.shutdown().await;
    Ok(())
}
