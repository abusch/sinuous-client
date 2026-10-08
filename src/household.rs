//! A whole household, with commands routed to the right player.

use std::{collections::HashMap, sync::Arc};

use iddqd::IdHashMap;
use tokio::sync::{Mutex, broadcast, watch};
use tracing::{debug, warn};
use url::Url;

use crate::{
    ConnectOptions, Connection, Error, Event, EventPayload, GroupHandle, GroupId, HouseholdId,
    PlayerHandle, PlayerId, Subscription,
    connection::lock,
    groups::{Group, Groups, Player},
};

const EVENT_CHANNEL_CAPACITY: usize = 256;

/// The groups and players in a household.
#[derive(Debug, Clone, Default)]
pub struct Topology {
    pub groups: IdHashMap<Group>,
    pub players: IdHashMap<Player>,
}

impl Topology {
    /// The player coordinating `group_id`.
    pub fn coordinator(&self, group_id: &GroupId) -> Option<&Player> {
        let group = self.groups.get(group_id)?;
        self.players.get(&group.coordinator_id)
    }

    /// The group `player_id` belongs to.
    pub fn group_of(&self, player_id: &PlayerId) -> Option<&Group> {
        self.groups
            .iter()
            .find(|group| group.player_ids.contains(player_id))
    }

    fn update(&mut self, groups: Groups) {
        if !groups.partial {
            *self = Self::default();
        }
        // A partial update only tells us about some of the household, so keep what we know.
        for group in groups.groups {
            self.groups.insert_overwrite(group);
        }
        for player in groups.players {
            self.players.insert_overwrite(player);
        }
    }
}

/// Control of a whole household through connections to its players.
///
/// The local API only accepts group commands on the websocket of the group's coordinator, and
/// player commands on the player's own websocket. A `Household` keeps track of the household's
/// [`Topology`] (by subscribing to `groups` events) and hands out [`GroupHandle`]s and
/// [`PlayerHandle`]s connected to the right player, opening connections as needed.
///
/// Household-scoped commands (favorites, playlists...) can be sent with
/// [`Household::connection`].
///
/// `Household` is cheap to clone.
#[derive(Debug, Clone)]
pub struct Household {
    inner: Arc<Inner>,
}

/// The connection to a player, if any. Locked while connecting, so that concurrent callers
/// don't open duplicate connections, without holding up commands to other players.
type ConnectionSlot = Mutex<Option<Connection>>;

#[derive(Debug)]
struct Inner {
    /// The connection the household was created from, used for household-scoped commands.
    primary: Connection,
    options: ConnectOptions,
    topology: Arc<watch::Sender<Topology>>,
    /// Connections to players, keyed by websocket URL.
    connections: std::sync::Mutex<HashMap<Url, Arc<ConnectionSlot>>>,
    /// Subscriptions made through the household, with the websocket URL of the player they were
    /// sent to.
    subscriptions: std::sync::Mutex<HashMap<Subscription, Url>>,
    events_tx: broadcast::Sender<Event>,
}

impl Household {
    /// Connect to the household of the player at `host` (a hostname or IP address).
    pub async fn connect(host: &str) -> Result<Self, Error> {
        Self::new(Connection::connect(host).await?).await
    }

    /// Build a household from a connection to any of its players.
    pub async fn new(conn: Connection) -> Result<Self, Error> {
        Self::with_options(conn, ConnectOptions::default()).await
    }

    /// Like [`Household::new`], with `options` used for connections to other players.
    pub async fn with_options(conn: Connection, options: ConnectOptions) -> Result<Self, Error> {
        let options = options.household_id(conn.household_id().clone());
        let topology = Arc::new(watch::Sender::new(Topology::default()));
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);

        forward_events(&conn, topology.clone(), events_tx.clone());
        let groups = conn.get_groups().await?;
        topology.send_modify(|topology| topology.update(groups));
        // Keep the topology up to date.
        conn.subscribe(&Subscription::Groups).await?;
        let subscriptions = HashMap::from([(Subscription::Groups, conn.websocket_url().clone())]);

        let connections = HashMap::from([(
            conn.websocket_url().clone(),
            Arc::new(Mutex::new(Some(conn.clone()))),
        )]);
        Ok(Self {
            inner: Arc::new(Inner {
                primary: conn,
                options,
                topology,
                connections: std::sync::Mutex::new(connections),
                subscriptions: std::sync::Mutex::new(subscriptions),
                events_tx,
            }),
        })
    }

    pub fn id(&self) -> &HouseholdId {
        self.inner.primary.household_id()
    }

    /// A connection for household-scoped commands, e.g. [`Connection::get_favorites`].
    pub fn connection(&self) -> &Connection {
        &self.inner.primary
    }

    /// A snapshot of the household's groups and players.
    pub fn topology(&self) -> Topology {
        self.inner.topology.borrow().clone()
    }

    /// Watch the household's topology as it changes.
    pub fn topology_updates(&self) -> watch::Receiver<Topology> {
        self.inner.topology.subscribe()
    }

    /// Receive events from all the players this household is connected to.
    ///
    /// Create the receiver *before* calling [`Household::subscribe`] to see the initial event
    /// players send on subscription.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.inner.events_tx.subscribe()
    }

    /// Commands for a group, sent to its coordinator.
    ///
    /// If the group changes coordinator between this call and a command, the command fails with
    /// [`Error::GroupCoordinatorChanged`]; get a new handle once the topology has caught up.
    pub async fn group(&self, id: &GroupId) -> Result<GroupHandle, Error> {
        let url = self.coordinator_url(id)?;
        Ok(self.connection_to(&url).await?.group(id))
    }

    /// Commands for a player, sent to the player itself.
    pub async fn player(&self, id: &PlayerId) -> Result<PlayerHandle, Error> {
        let url = self.player_url(id)?;
        Ok(self.connection_to(&url).await?.player(id))
    }

    /// Create a group from `player_ids`. The first player becomes its coordinator.
    ///
    /// The command is sent to the first player: other players reply *before* the group is
    /// actually created, so commands sent right after could be applied first. This also waits
    /// (up to the request timeout) for the topology to include the new group, so that
    /// [`Household::group`] can be used with it right away.
    pub async fn create_group(&self, player_ids: &[PlayerId]) -> Result<Group, Error> {
        let conn = match player_ids.first() {
            Some(first) => self.connection_to(&self.player_url(first)?).await?,
            // Let the player reject it.
            None => self.inner.primary.clone(),
        };
        let group = conn.create_group(player_ids).await?;

        let mut members = group.player_ids.clone();
        members.sort();
        let mut topology = self.inner.topology.subscribe();
        let known = topology.wait_for(|topology| {
            topology.groups.get(&group.id).is_some_and(|g| {
                let mut known_members = g.player_ids.clone();
                known_members.sort();
                known_members == members
            })
        });
        if tokio::time::timeout(self.inner.options.request_timeout, known)
            .await
            .is_err()
        {
            debug!("Topology did not catch up with new group {}", group.id);
        }
        Ok(group)
    }

    /// Subscribe to events, sending the subscription to the right player.
    ///
    /// Events are delivered to [`Household::events`]. The household subscribes to
    /// [`Subscription::Groups`] itself, to keep the topology up to date.
    pub async fn subscribe(&self, subscription: &Subscription) -> Result<(), Error> {
        let conn = self.connection_for(subscription).await?;
        conn.subscribe(subscription).await?;
        lock(&self.inner.subscriptions).insert(subscription.clone(), conn.websocket_url().clone());
        Ok(())
    }

    /// Stop receiving events for a subscription.
    ///
    /// The subscription is forgotten even if the command fails.
    pub async fn unsubscribe(&self, subscription: &Subscription) -> Result<(), Error> {
        let url = lock(&self.inner.subscriptions).remove(subscription);
        let conn = match url {
            // A closed connection has no subscriptions left, so there is nothing to undo.
            Some(url) => match self.open_connection(&url).await {
                Some(conn) => conn,
                None => return Ok(()),
            },
            // Not made through the household (or not anymore): send it where it would have been.
            None => self.connection_for(subscription).await?,
        };
        conn.unsubscribe(subscription).await
    }

    /// The active subscriptions, including [`Subscription::Groups`].
    pub fn subscriptions(&self) -> Vec<Subscription> {
        lock(&self.inner.subscriptions).keys().cloned().collect()
    }

    /// Close all connections.
    pub async fn close(&self) {
        let slots: Vec<_> = lock(&self.inner.connections).drain().collect();
        for (_, slot) in slots {
            if let Some(conn) = slot.lock().await.take() {
                conn.close().await;
            }
        }
    }

    fn coordinator_url(&self, group_id: &GroupId) -> Result<Url, Error> {
        let topology = self.inner.topology.borrow();
        let group = topology
            .groups
            .get(group_id)
            .ok_or_else(|| Error::UnknownGroup(group_id.clone()))?;
        let coordinator = topology
            .players
            .get(&group.coordinator_id)
            .ok_or_else(|| Error::UnknownPlayer(group.coordinator_id.clone()))?;
        Ok(coordinator.websocket_url.clone())
    }

    fn player_url(&self, player_id: &PlayerId) -> Result<Url, Error> {
        self.inner
            .topology
            .borrow()
            .players
            .get(player_id)
            .map(|player| player.websocket_url.clone())
            .ok_or_else(|| Error::UnknownPlayer(player_id.clone()))
    }

    async fn connection_for(&self, subscription: &Subscription) -> Result<Connection, Error> {
        let url = match subscription {
            Subscription::Groups | Subscription::Favorites | Subscription::Playlists => {
                return Ok(self.inner.primary.clone());
            }
            Subscription::Playback(id)
            | Subscription::PlaybackMetadata(id)
            | Subscription::GroupVolume(id) => self.coordinator_url(id)?,
            Subscription::PlayerVolume(id) | Subscription::HomeTheater(id) => {
                self.player_url(id)?
            }
        };
        self.connection_to(&url).await
    }

    /// The open connection to `url`, if any.
    async fn open_connection(&self, url: &Url) -> Option<Connection> {
        let slot = lock(&self.inner.connections).get(url)?.clone();
        let conn = slot.lock().await;
        conn.as_ref().filter(|conn| !conn.is_closed()).cloned()
    }

    /// Get the open connection to `url`, or open one.
    async fn connection_to(&self, url: &Url) -> Result<Connection, Error> {
        let slot = lock(&self.inner.connections)
            .entry(url.clone())
            .or_default()
            .clone();
        let mut slot = slot.lock().await;
        if let Some(conn) = slot.as_ref()
            && !conn.is_closed()
        {
            return Ok(conn.clone());
        }
        let conn = Connection::connect_to(url, self.inner.options.clone()).await?;
        forward_events(
            &conn,
            self.inner.topology.clone(),
            self.inner.events_tx.clone(),
        );
        *slot = Some(conn.clone());
        Ok(conn)
    }
}

/// Forward a connection's events to the household, updating the topology on the way.
///
/// The task ends when the connection is closed or dropped.
fn forward_events(
    conn: &Connection,
    topology: Arc<watch::Sender<Topology>>,
    events_tx: broadcast::Sender<Event>,
) {
    let mut events = conn.events();
    let url = conn.websocket_url().clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let EventPayload::Groups(groups) = &event.payload {
                        debug!("Updating topology");
                        topology.send_modify(|topology| topology.update(groups.clone()));
                    }
                    // Fails only if nobody is listening.
                    let _ = events_tx.send(event);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Dropped {n} events from {url}");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::Duration};

    use serde_json::{Value, json};

    use super::*;
    use crate::test_support::{self, FakePlayer, reply};

    /// Commands received by a fake player, as `namespace:command:target`.
    type Log = Arc<Mutex<Vec<String>>>;

    fn player(id: &str, url: &Url) -> Value {
        json!({
            "_objectType": "player", "id": id, "name": id, "websocketUrl": url.as_str(),
            "apiVersion": "1.54.1", "minApiVersion": "1.1.0", "softwareVersion": "97.1-80312",
            "capabilities": ["PLAYBACK"], "isUnregistered": false, "deviceIds": [id], "devices": []
        })
    }

    fn group(id: &str, name: &str, coordinator: &str) -> Value {
        json!({
            "_objectType": "group", "id": id, "name": name, "coordinatorId": coordinator,
            "playerIds": [coordinator]
        })
    }

    /// Answers `getGroups` with `topology`, sends `updated_topology` as a `groups` event on
    /// subscription, sends a `groupVolume` event on `groupVolume` subscription, and acknowledges
    /// everything else.
    fn handler(
        log: Log,
        topology: Value,
        updated_topology: Value,
    ) -> impl Fn(&Value, &Value) -> Option<Vec<Value>> + Send + Sync + 'static {
        move |header, body| {
            let namespace = header["namespace"].as_str().unwrap();
            let command = header["command"].as_str().unwrap();
            let target = ["groupId", "playerId", "householdId"]
                .iter()
                .find_map(|key| header[key].as_str())
                .unwrap_or_default();
            log.lock()
                .unwrap()
                .push(format!("{namespace}:{command}:{target}"));
            let mut replies = match command {
                "getGroups" => reply(header, true, topology.clone()),
                "createGroup" => {
                    let ids = &body["playerIds"];
                    let coordinator = ids[0].as_str().unwrap();
                    let group = json!({
                        "_objectType": "group", "id": format!("{coordinator}:1"), "name": "New",
                        "coordinatorId": coordinator, "playerIds": ids
                    });
                    reply(
                        header,
                        true,
                        json!({"_objectType": "groupInfo", "group": group}),
                    )
                }
                _ => reply(header, true, json!({})),
            };
            match (namespace, command) {
                ("groups", "subscribe") => replies.push(json!([
                    {"namespace": "groups", "name": "groups", "householdId": target},
                    updated_topology
                ])),
                ("groupVolume", "subscribe") => replies.push(json!([
                    {"namespace": "groupVolume", "name": "groupVolume", "groupId": target},
                    {"_objectType": "groupVolume", "volume": 10, "muted": false}
                ])),
                _ => {}
            }
            Some(replies)
        }
    }

    /// A household of two players, `A` and `B`, each coordinating its own group (`A:1` and
    /// `B:1`). The household is created from a connection to `A`. Once subscribed to `groups`,
    /// group `A:1` gets renamed.
    async fn household() -> (Household, Log, Log) {
        let a = FakePlayer::bind().await;
        let b = FakePlayer::bind().await;
        let players = [player("A", &a.url()), player("B", &b.url())];
        let topology = json!({
            "_objectType": "groups",
            "groups": [group("A:1", "Kitchen", "A"), group("B:1", "Office", "B")],
            "players": players,
        });
        let updated_topology = json!({
            "_objectType": "groups",
            "groups": [group("A:1", "Kitchen (renamed)", "A"), group("B:1", "Office", "B")],
            "players": players,
        });
        let (log_a, log_b) = (Log::default(), Log::default());
        let options = ConnectOptions::default()
            .request_timeout(Duration::from_millis(500))
            .connect_timeout(Duration::from_millis(500));
        let url_a = a.url();
        a.serve(handler(
            log_a.clone(),
            topology.clone(),
            updated_topology.clone(),
        ));
        b.serve(handler(log_b.clone(), topology, updated_topology));

        let conn = Connection::connect_to(
            &url_a,
            options.clone().household_id(HouseholdId::new("Sonos_1")),
        )
        .await
        .unwrap();
        let household = Household::with_options(conn, options).await.unwrap();
        settle(&household).await;
        (household, log_a, log_b)
    }

    /// Wait for the `groups` event sent when the household subscribed to the topology to be
    /// applied, so that tests changing the topology don't race with it.
    async fn settle(household: &Household) {
        let renamed = |topology: &Topology| {
            topology
                .groups
                .get(&GroupId::new("A:1"))
                .is_some_and(|group| group.name == "Kitchen (renamed)")
        };
        let mut topology = household.topology_updates();
        tokio::time::timeout(Duration::from_secs(2), topology.wait_for(renamed))
            .await
            .expect("topology was not updated")
            .unwrap();
    }

    fn logged(log: &Log) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn routes_commands_to_the_right_player() {
        let (household, log_a, log_b) = household().await;

        household
            .group(&GroupId::new("A:1"))
            .await
            .unwrap()
            .play()
            .await
            .unwrap();
        household
            .group(&GroupId::new("B:1"))
            .await
            .unwrap()
            .set_volume(10)
            .await
            .unwrap();
        // Reuses the connection to B (the fake player only accepts one).
        household
            .player(&PlayerId::new("B"))
            .await
            .unwrap()
            .set_mute(true)
            .await
            .unwrap();

        assert_eq!(
            logged(&log_a),
            [
                "groups:getGroups:Sonos_1",
                "groups:subscribe:Sonos_1",
                "playback:play:A:1"
            ]
        );
        assert_eq!(
            logged(&log_b),
            ["groupVolume:setVolume:B:1", "playerVolume:setMute:B"]
        );
    }

    #[tokio::test]
    async fn connects_to_players_independently() {
        let (household, _, log_b) = household().await;
        let (url_c, _server) = test_support::stalled_player().await;
        household.inner.topology.send_modify(|topology| {
            topology.update(
                serde_json::from_value(json!({
                    "groups": [group("C:1", "Garage", "C")],
                    "players": [player("C", &url_c)],
                    "partial": true,
                }))
                .unwrap(),
            )
        });

        let stuck = tokio::spawn({
            let household = household.clone();
            async move { household.player(&PlayerId::new("C")).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let set_volume = async {
            let group = household.group(&GroupId::new("B:1")).await?;
            group.set_volume(10).await
        };
        tokio::time::timeout(Duration::from_millis(200), set_volume)
            .await
            .expect("held up by the connection to C")
            .unwrap();
        assert_eq!(logged(&log_b), ["groupVolume:setVolume:B:1"]);
        let err = stuck.await.unwrap().unwrap_err();
        assert!(matches!(err, Error::Connect(_)), "{err:?}");
    }

    #[tokio::test]
    async fn creates_groups_through_the_new_coordinator() {
        let (household, log_a, log_b) = household().await;
        // `B:1` with just `B` is already known, so there is nothing to wait for.
        let start = tokio::time::Instant::now();
        let group = household.create_group(&[PlayerId::new("B")]).await.unwrap();
        assert_eq!(group.id, GroupId::new("B:1"));
        assert!(start.elapsed() < Duration::from_millis(400));
        assert_eq!(logged(&log_b), ["groups:createGroup:Sonos_1"]);
        assert!(!logged(&log_a).iter().any(|c| c.contains("createGroup")));
    }

    #[tokio::test]
    async fn waits_for_new_groups_up_to_the_request_timeout() {
        let (household, _, _) = household().await;
        let mut updates = household.topology_updates();
        // The fake players never report `B:1` with both players.
        let start = tokio::time::Instant::now();
        let group = household
            .create_group(&[PlayerId::new("B"), PlayerId::new("A")])
            .await
            .unwrap();
        assert_eq!(group.player_ids.len(), 2);
        assert!(start.elapsed() >= Duration::from_millis(450));
        assert!(updates.borrow_and_update().groups.get(&group.id).is_some());
    }

    #[tokio::test]
    async fn rejects_unknown_groups_and_players() {
        let (household, _, _) = household().await;
        let err = household.group(&GroupId::new("C:1")).await.unwrap_err();
        assert!(matches!(err, Error::UnknownGroup(_)), "{err:?}");
        let err = household.player(&PlayerId::new("C")).await.unwrap_err();
        assert!(matches!(err, Error::UnknownPlayer(_)), "{err:?}");
    }

    #[tokio::test]
    async fn merges_events_from_all_players() {
        let (household, _, log_b) = household().await;
        let mut events = household.events();
        household
            .subscribe(&Subscription::GroupVolume(GroupId::new("B:1")))
            .await
            .unwrap();
        assert_eq!(logged(&log_b), ["groupVolume:subscribe:B:1"]);

        // The `groups` event from `A`, sent when the household subscribed to the topology, may
        // still be in flight: skip it.
        let event = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let event = events.recv().await.unwrap();
                if matches!(event.payload, EventPayload::GroupVolume(_)) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(event.group_id, Some(GroupId::new("B:1")));
        assert!(matches!(event.payload, EventPayload::GroupVolume(v) if v.volume == 10));
    }

    #[tokio::test]
    async fn tracks_subscriptions() {
        let (household, log_a, log_b) = household().await;
        assert_eq!(household.subscriptions(), [Subscription::Groups]);
        let volume = Subscription::GroupVolume(GroupId::new("B:1"));
        household.subscribe(&volume).await.unwrap();
        household.subscribe(&Subscription::Favorites).await.unwrap();
        let mut subscriptions = household.subscriptions();
        subscriptions.sort_by_key(|s| format!("{s:?}"));
        assert_eq!(
            subscriptions,
            [
                Subscription::Favorites,
                volume.clone(),
                Subscription::Groups
            ]
        );

        household.unsubscribe(&volume).await.unwrap();
        household
            .unsubscribe(&Subscription::Favorites)
            .await
            .unwrap();
        assert_eq!(household.subscriptions(), [Subscription::Groups]);
        assert_eq!(
            logged(&log_b),
            ["groupVolume:subscribe:B:1", "groupVolume:unsubscribe:B:1"]
        );
        assert_eq!(
            logged(&log_a)[2..],
            [
                "favorites:subscribe:Sonos_1",
                "favorites:unsubscribe:Sonos_1"
            ]
        );
    }

    #[tokio::test]
    async fn does_not_track_failed_subscriptions() {
        let (household, _, _) = household().await;
        let err = household
            .subscribe(&Subscription::Playback(GroupId::new("C:1")))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::UnknownGroup(_)), "{err:?}");
        assert_eq!(household.subscriptions(), [Subscription::Groups]);
    }

    #[tokio::test]
    async fn updates_topology_from_groups_events() {
        let (household, _, _) = household().await;
        let name = || {
            household
                .topology()
                .groups
                .get(&GroupId::new("A:1"))
                .unwrap()
                .name
                .clone()
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            while name() != "Kitchen (renamed)" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("topology was not updated");
    }

    #[test]
    fn partial_updates_keep_known_groups() {
        let decode = |value: Value| serde_json::from_value::<Groups>(value).unwrap();
        let url = Url::parse("wss://10.0.0.1:1443/websocket/api").unwrap();
        let mut topology = Topology::default();
        topology.update(decode(json!({
            "groups": [group("A:1", "Kitchen", "A"), group("B:1", "Office", "B")],
            "players": [player("A", &url), player("B", &url)],
        })));
        topology.update(decode(json!({
            "groups": [group("A:1", "Kitchen (renamed)", "A")],
            "players": [player("A", &url)],
            "partial": true,
        })));
        assert_eq!(topology.groups.len(), 2);
        assert_eq!(
            topology.coordinator(&GroupId::new("B:1")).unwrap().id,
            PlayerId::new("B")
        );

        topology.update(decode(json!({
            "groups": [group("A:1", "Kitchen", "A")],
            "players": [player("A", &url)],
        })));
        assert_eq!(topology.groups.len(), 1);
        assert!(topology.group_of(&PlayerId::new("B")).is_none());
    }
}
