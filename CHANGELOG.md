# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/abusch/sinuous-client/commits/main
