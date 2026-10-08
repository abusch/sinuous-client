//! Show the state of a household: players, groups, volumes, what's playing, favorites and
//! playlists, then print events from all groups for a few seconds.
//!
//! Only reads state, it doesn't change anything.
//!
//! ```sh
//! cargo run --example household              # discover players with SSDP
//! cargo run --example household 10.0.0.42    # or connect to a known player
//! RUST_LOG=sonos_ws=debug cargo run --example household
//! ```

use std::time::Duration;

use sonos_ws::{EventPayload, Household, Subscription};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("sonos_ws=info")),
        )
        .init();
    let household = match std::env::args().nth(1) {
        Some(host) => Household::connect(&host).await?,
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
            Household::new(player.connect().await?).await?
        }
    };
    println!("Connected to household {}", household.id());

    let topology = household.topology();
    for p in &topology.players {
        println!("Found player {}", p.name);
        for d in &p.devices {
            println!("\tDevice {} ({})", d.name, d.model_display_name);
        }
    }

    for g in &topology.groups {
        let coordinator = topology.coordinator(&g.id).map(|p| p.name.as_str());
        println!(
            "\nGroup {} ({:?}), coordinator = {coordinator:?}",
            g.name, g.playback_state
        );
        let group = household.group(&g.id).await?;
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
    }

    for p in &topology.players {
        let volume = household.player(&p.id).await?.get_volume().await?;
        println!("Player {} volume: {}", p.name, volume.volume);
    }

    let conn = household.connection();
    let favorites = conn.get_favorites().await?;
    println!("\nFavorites:");
    for f in &favorites.items {
        println!("\t{} [{}]", f.name, f.id);
    }
    let playlists = conn.get_playlists().await?;
    println!("Playlists:");
    for p in &playlists.playlists {
        let playlist = conn.get_playlist(&p.id).await?;
        println!("\t{} [{}]: {} tracks", p.name, p.id, playlist.tracks.len());
    }

    // Events from every group, whichever player coordinates it.
    println!("\nListening to events for all groups for 5s...");
    let mut events = household.events();
    for g in &topology.groups {
        household
            .subscribe(&Subscription::Playback(g.id.clone()))
            .await?;
        household
            .subscribe(&Subscription::GroupVolume(g.id.clone()))
            .await?;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(event) = events.recv().await {
            let group = event
                .group_id
                .as_ref()
                .and_then(|id| topology.groups.get(id))
                .map(|g| g.name.as_str());
            let summary = match &event.payload {
                EventPayload::PlaybackStatus(s) => format!("{:?}", s.playback_state),
                EventPayload::GroupVolume(v) => format!("volume {}", v.volume),
                other => format!("{other:?}"),
            };
            println!("\t{group:?} {}/{}: {summary}", event.namespace, event.name);
        }
    })
    .await;

    household.close().await;
    Ok(())
}
