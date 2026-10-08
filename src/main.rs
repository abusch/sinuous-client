use std::time::Duration;

use iddqd::IdHashMap;
use sonos_ws::{ConnectOptions, Connection, Subscription};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("sonos_ws=info")),
        )
        .init();
    let sonos = match std::env::args().nth(1) {
        Some(host) => Connection::connect(&host).await?,
        None => {
            let players = sonos_ws::discover(Duration::from_secs(2)).await?;
            println!("Discovered {} players:", players.len());
            for p in &players {
                let group = p.group.as_ref();
                println!(
                    "\t{} at {} (group {:?}{})",
                    p.player_id,
                    p.address,
                    group.map(|g| &g.name),
                    if group.is_some_and(|g| g.is_coordinator) {
                        ", coordinator"
                    } else {
                        ""
                    },
                );
            }
            let Some(player) = players.first() else {
                anyhow::bail!("No players found");
            };
            player.connect().await?
        }
    };
    println!(
        "Connected to {} (household {})",
        sonos.websocket_url(),
        sonos.household_id()
    );

    let groups = sonos.get_groups().await?;
    let mut group_map = IdHashMap::new();
    let mut player_map = IdHashMap::new();
    for g in groups.groups {
        group_map.insert_unique(g).expect("Duplicate group!");
    }
    for p in groups.players {
        player_map.insert_unique(p).expect("Duplicate player!");
    }

    for p in &player_map {
        println!("Found player {}", p.name);
        for d in &p.devices {
            println!("\tDevice {} ({})", d.name, d.model_display_name);
        }
    }

    for g in &group_map {
        let Some(coordinator) = player_map.get(&g.coordinator_id) else {
            println!("Missing coordinator for group {}!", g.name);
            continue;
        };
        println!(
            "\nGroup {} ({:?}), coordinator = {}",
            g.name, g.playback_state, coordinator.name
        );

        // Group commands must be sent to the coordinator.
        let is_remote = coordinator.websocket_url != *sonos.websocket_url();
        let conn = if !is_remote {
            sonos.clone()
        } else {
            let options = ConnectOptions::default().household_id(sonos.household_id().clone());
            Connection::connect_to(&coordinator.websocket_url, options).await?
        };
        let group = conn.group(&g.id);
        let volume = group.get_volume().await?;
        let status = group.get_playback_status().await?;
        let metadata = group.get_metadata_status().await?;
        println!(
            "\tvolume {}{}, {:?} at {:?}ms",
            volume.volume,
            if volume.muted { " (muted)" } else { "" },
            status.playback_state,
            status.position_millis,
        );
        if let Some(track) = metadata.current_item.map(|item| item.track) {
            println!(
                "\tcurrent track: {} - {}",
                track.artist.map(|a| a.name).unwrap_or_default(),
                track.name.unwrap_or_default(),
            );
        }
        if is_remote {
            conn.close().await;
        }
    }

    let favorites = sonos.get_favorites().await?;
    println!("\nFavorites:");
    for f in &favorites.items {
        println!("\t{} [{}]", f.name, f.id);
    }
    let playlists = sonos.get_playlists().await?;
    println!("Playlists:");
    for p in &playlists.playlists {
        let playlist = sonos.get_playlist(&p.id).await?;
        println!("\t{} [{}]: {} tracks", p.name, p.id, playlist.tracks.len());
    }

    // Listen to events for the group coordinated by the player we're connected to.
    if let Some(group) = group_map.iter().find(|g| {
        player_map.get(&g.coordinator_id).map(|p| &p.websocket_url) == Some(sonos.websocket_url())
    }) {
        println!("\nListening to events for {} for 5s...", group.name);
        let mut events = sonos.events();
        for sub in [
            Subscription::Playback(group.id.clone()),
            Subscription::PlaybackMetadata(group.id.clone()),
            Subscription::GroupVolume(group.id.clone()),
        ] {
            sonos.subscribe(&sub).await?;
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(event) = events.recv().await {
                println!("\t{}/{}: {:?}", event.namespace, event.name, event.payload);
            }
        })
        .await;
    }

    sonos.close().await;
    Ok(())
}
