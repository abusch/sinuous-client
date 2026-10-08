# sinuous-client

An unofficial Rust client for controlling Sonos speakers on your local network, through the
websocket API the players expose. It's the library behind
[sinuous](https://github.com/abusch/sinuous), a TUI for Sonos.

> [!WARNING]
> This uses an **undocumented** API. It resembles the
> [Sonos Control API](https://docs.sonos.com/docs/control), but differs from it in places, and
> it could change or stop working with any firmware update.
>
> This project is not affiliated with or endorsed by Sonos, Inc.

## Features

- Find players on the network (SSDP)
- Control groups, playback, volume, favorites, playlists and home theater settings
- Receive events, e.g. when playback or volume changes
- Send each command to the player that accepts it: group commands only work on the group's
  coordinator, and player commands on the player itself
- Reconnect to players when a connection is lost, and restore its event subscriptions

## Example

```rust,no_run
use std::time::Duration;

use sinuous_client::{Household, discover};

#[tokio::main]
async fn main() -> Result<(), sinuous_client::Error> {
    // Pick the crypto provider rustls uses to talk to the players.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("a crypto provider is already installed");

    let players = discover(Duration::from_secs(2)).await?;
    let player = players.first().expect("no players found");
    let household = Household::new(player.connect().await?).await?;

    for group in &household.topology().groups {
        let status = household.group(&group.id).await?.get_playback_status().await?;
        println!("{}: {:?}", group.name, status.playback_state);
    }
    Ok(())
}
```

The players are reached over TLS, using [rustls](https://docs.rs/rustls). As a library, this
crate leaves the choice of crypto provider to your application: install one as the
[process-wide default](https://docs.rs/rustls/latest/rustls/crypto/struct.CryptoProvider.html#using-the-per-process-default-cryptoprovider)
as above, or enable exactly one of rustls' `aws_lc_rs` and `ring` features.

`cargo run --example household` prints a summary of the household it finds.

## License

MIT
