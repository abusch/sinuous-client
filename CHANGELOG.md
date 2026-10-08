# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/abusch/sinuous-client/compare/v0.1.1...v0.2.0) - 2026-10-08

### Added

- *(household)* follow subscriptions when a group's coordinator changes
- *(household)* [**breaking**] reconnect and restore subscriptions when a connection drops
- *(household)* keep track of subscriptions
- *(connection)* add Connection::closed to wait for the websocket to close
- *(connection)* time out when connecting to a player

### Fixed

- *(household)* don't hold up commands to other players while connecting to one

## [0.1.1](https://github.com/abusch/sinuous-client/compare/v0.1.0...v0.1.1) - 2026-10-08

### Added

- *(discovery)* add discover_stream to get players as they answer

### Dependencies

- *(deps)* bump tokio-tungstenite from 0.29.0 to 0.30.0

## [0.1.0](https://github.com/abusch/sinuous-client/releases/tag/v0.1.0) - 2026-10-08

Initial release.

### Added

- Discovery of players on the local network with SSDP (`discover`).
- `Connection` to a single player over its local websocket API, with typed commands for the
  `groups`, `groupVolume`, `playerVolume`, `playback`, `playbackMetadata`, `favorites`,
  `playlists` and `homeTheater` namespaces.
- Event subscriptions, e.g. for playback, metadata and volume changes.
- `Household`, which tracks the household's topology and sends group commands to the group's
  coordinator and player commands to the player itself.
- TLS connections that verify player certificates against the Sonos root CA. The crypto
  provider is left to the application: install a process-wide default `CryptoProvider`, or enable
  exactly one of rustls' `aws_lc_rs` and `ring` features.
