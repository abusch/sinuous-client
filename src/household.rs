//! A whole household, with commands routed to the right player.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
    time::Duration,
};

use iddqd::IdHashMap;
use tokio::sync::{Mutex, broadcast, watch};
use tracing::{debug, info, warn};
use url::Url;

use crate::{
    ConnectOptions, Connection, Error, Event, EventPayload, GroupHandle, GroupId, HouseholdId,
    PlayerHandle, PlayerId, Subscription,
    connection::lock,
    groups::{Group, GroupStatus, Groups, Player},
};

const EVENT_CHANNEL_CAPACITY: usize = 256;
/// How long to wait before retrying to restore subscriptions after a lost connection. Doubles
/// after each failure, up to [`RECONNECT_MAX_DELAY`].
const RECONNECT_MIN_DELAY: Duration = Duration::from_millis(500);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

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
/// # Lost connections
///
/// When the connection to a player is lost (e.g. the player restarts, or the network drops),
/// the household reconnects to it in the background and restores the subscriptions that were
/// sent to it, retrying with exponential backoff until it succeeds. As on subscription, players
/// send events with their current state when the subscriptions are restored, including a
/// `groups` event that brings the topology up to date.
///
/// A player that is restarting may leave the topology for a while, so its subscriptions are
/// kept until it comes back. Subscriptions are dropped (and no longer listed by
/// [`Household::subscriptions`]) only when the player rejects them, or when their group is gone
/// while the player that coordinated it is back.
///
/// Group-scoped subscriptions also follow their group: when its coordinator changes, they are
/// sent to the new coordinator, and they are dropped when the group is gone, e.g. after its
/// players joined other groups. Either way, the `groupCoordinatorChanged` event is still
/// delivered.
///
/// Handles hold on to the connection they were created with, so commands sent through a handle
/// obtained before the connection was lost fail with [`Error::ConnectionClosed`]. Get a new
/// handle from the household instead: handles are cheap, and it reconnects as needed.
///
/// `Household` is cheap to clone. Background reconnections stop when it is closed or the last
/// clone is dropped.
#[derive(Debug, Clone)]
pub struct Household {
    inner: Arc<Inner>,
}

/// The connection to a player, if any. Locked while connecting, so that concurrent callers
/// don't open duplicate connections, without holding up commands to other players.
type ConnectionSlot = Mutex<Option<Connection>>;

#[derive(Debug)]
struct Inner {
    household_id: HouseholdId,
    /// The player the household was created from, used for household-scoped commands.
    primary_url: Url,
    options: ConnectOptions,
    topology: Arc<watch::Sender<Topology>>,
    /// Connections to players, keyed by websocket URL.
    connections: std::sync::Mutex<HashMap<Url, Arc<ConnectionSlot>>>,
    /// Subscriptions made through the household, with where they were sent.
    subscriptions: std::sync::Mutex<HashMap<Subscription, SentTo>>,
    events_tx: broadcast::Sender<Event>,
    /// Set once the household is closed. Dropping it also tells the tasks restoring
    /// subscriptions that the household is gone.
    closed: watch::Sender<bool>,
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

    /// Like [`Household::new`], with `options` used for connections to other players, and to
    /// reconnect to this one.
    pub async fn with_options(conn: Connection, options: ConnectOptions) -> Result<Self, Error> {
        let household_id = conn.household_id().clone();
        let primary_url = conn.websocket_url().clone();
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let household = Self {
            inner: Arc::new(Inner {
                options: options.household_id(household_id.clone()),
                household_id,
                topology: Arc::new(watch::Sender::new(Topology::default())),
                connections: std::sync::Mutex::new(HashMap::from([(
                    primary_url.clone(),
                    Arc::new(Mutex::new(Some(conn.clone()))),
                )])),
                primary_url,
                subscriptions: std::sync::Mutex::default(),
                events_tx,
                closed: watch::Sender::new(false),
            }),
        };

        household.watch(&conn);
        let groups = conn.get_groups().await?;
        household
            .inner
            .topology
            .send_modify(|topology| topology.update(groups));
        // Keep the topology up to date.
        household.subscribe(&Subscription::Groups).await?;
        Ok(household)
    }

    pub fn id(&self) -> &HouseholdId {
        &self.inner.household_id
    }

    /// A connection for household-scoped commands, e.g. [`Connection::get_favorites`].
    ///
    /// This is the connection the household was created from, or a new connection to the same
    /// player if it was lost.
    pub async fn connection(&self) -> Result<Connection, Error> {
        self.connection_to(&self.inner.primary_url).await
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
            None => self.connection().await?,
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
    ///
    /// The subscription is restored if the connection to the player is lost, and follows its
    /// group if the group moves; see [Lost connections](Household#lost-connections).
    pub async fn subscribe(&self, subscription: &Subscription) -> Result<(), Error> {
        let conn = self.connection_to(&self.route(subscription)?).await?;
        conn.subscribe(subscription).await?;
        lock(&self.inner.subscriptions).insert(subscription.clone(), SentTo::new(&conn));
        self.restore_if_closed(&conn);
        Ok(())
    }

    /// Stop receiving events for a subscription.
    ///
    /// The subscription is forgotten even if the command fails.
    pub async fn unsubscribe(&self, subscription: &Subscription) -> Result<(), Error> {
        let sent_to = lock(&self.inner.subscriptions).remove(subscription);
        let conn = match sent_to {
            Some(SentTo {
                url,
                connection: Some(id),
            }) => match self.open_connection(&url).await {
                // Only the connection it was sent on has it: a closed connection has no
                // subscriptions left, and a newer one never had it.
                Some(conn) if conn.id() == id => conn,
                _ => return Ok(()),
            },
            // Being restored: undone once it is.
            Some(SentTo {
                connection: None, ..
            }) => return Ok(()),
            // Not made through the household (or not anymore): send it where it would have been.
            None => self.connection_to(&self.route(subscription)?).await?,
        };
        conn.unsubscribe(subscription).await
    }

    /// The active subscriptions, including [`Subscription::Groups`] and the ones being restored.
    ///
    /// Subscriptions to groups that are gone are removed.
    pub fn subscriptions(&self) -> Vec<Subscription> {
        lock(&self.inner.subscriptions).keys().cloned().collect()
    }

    /// Close all connections.
    ///
    /// Commands fail with [`Error::ConnectionClosed`] afterwards, and lost connections are no
    /// longer restored.
    pub async fn close(&self) {
        self.inner.closed.send_replace(true);
        lock(&self.inner.subscriptions).clear();
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

    /// The websocket URL of the player that accepts `subscription`.
    fn route(&self, subscription: &Subscription) -> Result<Url, Error> {
        match subscription {
            Subscription::Groups | Subscription::Favorites | Subscription::Playlists => {
                Ok(self.inner.primary_url.clone())
            }
            Subscription::Playback(id)
            | Subscription::PlaybackMetadata(id)
            | Subscription::GroupVolume(id) => self.coordinator_url(id),
            Subscription::PlayerVolume(id) | Subscription::HomeTheater(id) => self.player_url(id),
        }
    }

    /// The open connection to `url`, if any.
    async fn open_connection(&self, url: &Url) -> Option<Connection> {
        let slot = lock(&self.inner.connections).get(url)?.clone();
        let conn = slot.lock().await;
        conn.as_ref().filter(|conn| !conn.is_closed()).cloned()
    }

    /// Get the open connection to `url`, or open one.
    async fn connection_to(&self, url: &Url) -> Result<Connection, Error> {
        // Don't leave a slot behind once closed.
        if *self.inner.closed.borrow() {
            return Err(Error::ConnectionClosed);
        }
        let slot = lock(&self.inner.connections)
            .entry(url.clone())
            .or_default()
            .clone();
        let mut slot = slot.lock().await;
        // Checked again with the slot locked, so that `close` waits for connections being opened
        // and closes them too.
        if *self.inner.closed.borrow() {
            return Err(Error::ConnectionClosed);
        }
        if let Some(conn) = slot.as_ref()
            && !conn.is_closed()
        {
            return Ok(conn.clone());
        }
        let conn = Connection::connect_to(url, self.inner.options.clone()).await?;
        self.watch(&conn);
        *slot = Some(conn.clone());
        Ok(conn)
    }

    /// Forward a connection's events to the household, updating the topology on the way, and
    /// follow subscribed groups that move. Once the connection is closed, restore the
    /// subscriptions that were sent on it.
    ///
    /// The task doesn't keep the household (or the connection) alive.
    fn watch(&self, conn: &Connection) {
        let mut events = conn.events();
        let closed = conn.closed();
        let (url, id) = (conn.websocket_url().clone(), conn.id());
        let topology = self.inner.topology.clone();
        let events_tx = self.inner.events_tx.clone();
        let household = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut closed = std::pin::pin!(closed);
            loop {
                tokio::select! {
                    // Forward the events received before the connection was closed first.
                    biased;
                    event = events.recv() => match event {
                        Ok(event) => {
                            match (&event.payload, &event.group_id) {
                                (EventPayload::Groups(groups), _) => {
                                    debug!("Updating topology");
                                    topology
                                        .send_modify(|topology| topology.update(groups.clone()));
                                }
                                (EventPayload::GroupCoordinatorChanged(changed), Some(group_id)) => {
                                    tokio::spawn(follow_group(
                                        household.clone(),
                                        id,
                                        group_id.clone(),
                                        changed.group_status,
                                    ));
                                }
                                _ => {}
                            }
                            // Fails only if nobody is listening.
                            let _ = events_tx.send(event);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("Dropped {n} events from {url}");
                        }
                        // The connection was dropped, which also closes it.
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                    () = &mut closed => break,
                }
            }
            restore_lost(household, url, id).await;
        });
    }

    /// Restore the subscriptions sent on `conn` if it is closed already.
    ///
    /// Called after recording a subscription: if the connection closed in the meantime, its
    /// watch task may have looked for subscriptions to restore before this one was recorded.
    fn restore_if_closed(&self, conn: &Connection) {
        if conn.is_closed() {
            tokio::spawn(restore_lost(
                Arc::downgrade(&self.inner),
                conn.websocket_url().clone(),
                conn.id(),
            ));
        }
    }

    /// Send `subscriptions` again, after the connection they were sent on was lost or their group
    /// moved. Returns the ones that failed and should be retried.
    async fn resubscribe(&self, subscriptions: Vec<Subscription>) -> Vec<Subscription> {
        let mut failed = Vec::new();
        // Players that couldn't be reached this time, so as not to wait for each of their
        // subscriptions to time out.
        let mut unreachable = HashSet::new();
        for subscription in subscriptions {
            let Some(sent_to) = lock(&self.inner.subscriptions).get(&subscription).cloned() else {
                // Unsubscribed in the meantime.
                continue;
            };
            let url = match self.route(&subscription) {
                Ok(url) => url,
                Err(e) if self.is_gone_from_topology(&e, &sent_to) => {
                    warn!("Dropping subscription {subscription:?}: {e}");
                    lock(&self.inner.subscriptions).remove(&subscription);
                    continue;
                }
                // E.g. the player is restarting, so it left the topology for now.
                Err(e) => {
                    debug!("Can't restore subscription {subscription:?} yet: {e}");
                    failed.push(subscription);
                    continue;
                }
            };
            if unreachable.contains(&url) {
                failed.push(subscription);
                continue;
            }
            let conn = match self.connection_to(&url).await {
                Ok(conn) => conn,
                Err(e) => {
                    debug!("Failed to restore subscription {subscription:?}: {e}");
                    unreachable.insert(url);
                    failed.push(subscription);
                    continue;
                }
            };
            // Players send a `groups` event right after the reply, and the subscriptions restored
            // after this one are routed with the topology, so wait for it to be applied.
            let mut topology =
                (subscription == Subscription::Groups).then(|| self.inner.topology.subscribe());
            let result = match conn.subscribe(&subscription).await {
                Ok(()) => Ok(conn),
                // The topology hasn't caught up with the group's new coordinator yet: go where
                // the player says.
                Err(e) => match moved_to(&e) {
                    Some(url) => {
                        async {
                            let conn = self.connection_to(url).await?;
                            conn.subscribe(&subscription).await?;
                            Ok(conn)
                        }
                        .await
                    }
                    None => Err(e),
                },
            };
            match result {
                Ok(conn) => {
                    debug!("Restored subscription {subscription:?}");
                    let tracked = match lock(&self.inner.subscriptions).get_mut(&subscription) {
                        Some(sent_to) => {
                            *sent_to = SentTo::new(&conn);
                            true
                        }
                        None => false,
                    };
                    if !tracked {
                        // Unsubscribed while it was being restored: undo it.
                        let _ = conn.unsubscribe(&subscription).await;
                        continue;
                    }
                    self.restore_if_closed(&conn);
                    if let Some(topology) = &mut topology {
                        let timeout = self.inner.options.request_timeout;
                        let _ = tokio::time::timeout(timeout, topology.changed()).await;
                    }
                }
                Err(e) if is_rejection(&e) => {
                    warn!("Dropping subscription {subscription:?}: {e}");
                    lock(&self.inner.subscriptions).remove(&subscription);
                }
                Err(e) => {
                    debug!("Failed to restore subscription {subscription:?}: {e}");
                    failed.push(subscription);
                }
            }
        }
        failed
    }

    /// Whether `error`, from routing a subscription last sent to `sent_to`, means that its group
    /// is gone for good: the player coordinating it is still in the topology, but the group isn't.
    ///
    /// Otherwise the player may just be restarting, and the group may come back with it.
    fn is_gone_from_topology(&self, error: &Error, sent_to: &SentTo) -> bool {
        matches!(error, Error::UnknownGroup(_))
            && self
                .inner
                .topology
                .borrow()
                .players
                .iter()
                .any(|player| player.websocket_url == sent_to.url)
    }
}

/// Where a subscription was sent.
#[derive(Debug, Clone)]
struct SentTo {
    /// The websocket URL of the player.
    url: Url,
    /// The [ID](Connection::id) of the connection, or `None` while the subscription is being
    /// restored.
    connection: Option<u64>,
}

impl SentTo {
    fn new(conn: &Connection) -> Self {
        Self {
            url: conn.websocket_url().clone(),
            connection: Some(conn.id()),
        }
    }
}

impl Inner {
    /// Mark the subscriptions that `filter` selects among the ones sent on connection
    /// `connection` as being restored, and return them.
    ///
    /// Each subscription is only claimed once, so that it is restored only once even when
    /// several tasks look for it.
    fn claim(&self, connection: u64, filter: impl Fn(&Subscription) -> bool) -> Vec<Subscription> {
        lock(&self.subscriptions)
            .iter_mut()
            .filter(|(subscription, sent_to)| {
                sent_to.connection == Some(connection) && filter(subscription)
            })
            .map(|(subscription, sent_to)| {
                sent_to.connection = None;
                subscription.clone()
            })
            .collect()
    }
}

/// Where the group went, if `error` says it moved.
fn moved_to(error: &Error) -> Option<&Url> {
    match error {
        Error::GroupCoordinatorChanged(changed) if changed.group_status == GroupStatus::Moved => {
            changed.websocket_url.as_ref()
        }
        _ => None,
    }
}

/// Whether `error` means that the player won't accept the subscription: it rejected it, or the
/// group is gone.
fn is_rejection(error: &Error) -> bool {
    match error {
        Error::Api(_) => true,
        Error::GroupCoordinatorChanged(changed) => changed.group_status == GroupStatus::Gone,
        _ => false,
    }
}

/// Restore the subscriptions that were sent on connection `id` to `url`, after it was lost.
async fn restore_lost(household: Weak<Inner>, url: Url, id: u64) {
    let Some(inner) = household.upgrade() else {
        return;
    };
    let lost = inner.claim(id, |_| true);
    drop(inner);
    if lost.is_empty() {
        debug!("Connection to {url} closed");
        return;
    }
    warn!(
        "Lost connection to {url}, restoring {} subscriptions",
        lost.len()
    );
    restore(household, lost).await;
}

/// Follow the subscriptions to `group_id` that were sent on connection `id`, after its player
/// said that the group moved to another coordinator or disappeared.
async fn follow_group(household: Weak<Inner>, id: u64, group_id: GroupId, status: GroupStatus) {
    let Some(inner) = household.upgrade() else {
        return;
    };
    let in_group = |subscription: &Subscription| subscription.group_id() == Some(&group_id);
    match status {
        GroupStatus::Moved => {
            let moved = inner.claim(id, in_group);
            drop(inner);
            if !moved.is_empty() {
                info!(
                    "Group {group_id} moved, following {} subscriptions",
                    moved.len()
                );
                restore(household, moved).await;
            }
        }
        GroupStatus::Gone => {
            lock(&inner.subscriptions).retain(|subscription, sent_to| {
                let gone = sent_to.connection == Some(id) && in_group(subscription);
                if gone {
                    info!("Group {group_id} is gone, dropping subscription {subscription:?}");
                }
                !gone
            });
        }
        // The group's name or members changed: its subscriptions carry on.
        GroupStatus::Updated => {}
        GroupStatus::Unknown => debug!("Group {group_id} changed in an unknown way"),
    }
}

/// Send `subscriptions` again, which have been [claimed](Inner::claim).
///
/// Retries with exponential backoff until they are all restored (or dropped), or the household
/// is closed or dropped.
async fn restore(household: Weak<Inner>, mut subscriptions: Vec<Subscription>) {
    let Some(inner) = household.upgrade() else {
        return;
    };
    let mut closed = inner.closed.subscribe();
    drop(inner);
    // `Groups` first, so that the topology used to route the others is up to date.
    subscriptions.sort_by_key(|subscription| match subscription {
        Subscription::Groups => 0,
        Subscription::Favorites | Subscription::Playlists => 1,
        _ => 2,
    });

    let mut delay = RECONNECT_MIN_DELAY;
    loop {
        if *closed.borrow() {
            return;
        }
        let Some(inner) = household.upgrade() else {
            return;
        };
        subscriptions = Household { inner }.resubscribe(subscriptions).await;
        if subscriptions.is_empty() {
            info!("Restored subscriptions");
            return;
        }
        debug!("Retrying in {delay:?}");
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            // Fails if the household was dropped.
            _ = closed.wait_for(|closed| *closed) => return,
        }
        delay = (delay * 2).min(RECONNECT_MAX_DELAY);
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Mutex, time::Duration};

    use serde_json::{Value, json};

    use super::*;
    use crate::test_support::{self, FakePlayer, FakeServer, reply};

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

    /// How the fake players behave, shared by all of them so that tests can change it.
    #[derive(Default)]
    struct Behavior {
        /// Sent as a `groups` event on subscription.
        groups_event: Mutex<Value>,
        /// Commands that fail with a `globalError`, as `namespace:command:target`.
        rejected: Mutex<HashSet<String>>,
        /// Groups that moved to another coordinator, by ID, with the new coordinator's websocket
        /// URL.
        moves: Mutex<HashMap<String, Url>>,
    }

    /// A `groupCoordinatorChanged` event for group `B:1`.
    fn coordinator_changed(status: GroupStatus, new_url: Option<&Url>) -> Value {
        let status = match status {
            GroupStatus::Gone => "GROUP_STATUS_GONE",
            GroupStatus::Moved => "GROUP_STATUS_MOVED",
            GroupStatus::Updated => "GROUP_STATUS_UPDATED",
            GroupStatus::Unknown => unreachable!(),
        };
        json!([
            {"namespace": "global", "name": "groupCoordinatorChanged", "groupId": "B:1",
             "householdId": "Sonos_1"},
            {"_objectType": "groupCoordinatorChanged", "groupStatus": status,
             "groupName": "Office", "websocketUrl": new_url.map(Url::as_str)}
        ])
    }

    /// Answers `getGroups` with `topology`, sends a `groups` event on subscription, sends a
    /// `groupVolume` event on `groupVolume` subscription, and acknowledges everything else, as
    /// [`Behavior`] says.
    fn handler(
        me: Url,
        log: Log,
        behavior: Arc<Behavior>,
        topology: Value,
    ) -> impl Fn(&Value, &Value) -> Option<Vec<Value>> + Send + Sync + 'static {
        move |header, body| {
            let namespace = header["namespace"].as_str().unwrap();
            let command = header["command"].as_str().unwrap();
            let target = ["groupId", "playerId", "householdId"]
                .iter()
                .find_map(|key| header[key].as_str())
                .unwrap_or_default();
            let logged = format!("{namespace}:{command}:{target}");
            log.lock().unwrap().push(logged.clone());
            if behavior.rejected.lock().unwrap().contains(&logged) {
                let error = json!({"_objectType": "globalError", "errorCode": "ERROR_NOT_CAPABLE"});
                return Some(reply(header, false, error));
            }
            if let Some(group_id) = header["groupId"].as_str()
                && let Some(new_url) = behavior.moves.lock().unwrap().get(group_id)
                && *new_url != me
            {
                return Some(reply(
                    header,
                    false,
                    coordinator_changed(GroupStatus::Moved, Some(new_url))[1].clone(),
                ));
            }
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
                    behavior.groups_event.lock().unwrap().clone()
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

    struct Fixture {
        household: Household,
        a: FakeServer,
        b: FakeServer,
        log_a: Log,
        log_b: Log,
        behavior: Arc<Behavior>,
    }

    async fn household() -> (Household, Log, Log) {
        let fixture = fixture().await;
        (fixture.household, fixture.log_a, fixture.log_b)
    }

    /// A household of two players, `A` and `B`, each coordinating its own group (`A:1` and
    /// `B:1`). The household is created from a connection to `A`. Once subscribed to `groups`,
    /// group `A:1` gets renamed.
    async fn fixture() -> Fixture {
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
        let (url_a, url_b) = (a.url(), b.url());
        let behavior = Arc::new(Behavior {
            groups_event: Mutex::new(updated_topology),
            ..Behavior::default()
        });
        let a = a.serve_many(handler(
            url_a.clone(),
            log_a.clone(),
            behavior.clone(),
            topology.clone(),
        ));
        let b = b.serve_many(handler(url_b, log_b.clone(), behavior.clone(), topology));

        let conn = Connection::connect_to(
            &url_a,
            options.clone().household_id(HouseholdId::new("Sonos_1")),
        )
        .await
        .unwrap();
        let household = Household::with_options(conn, options).await.unwrap();
        settle(&household).await;
        Fixture {
            household,
            a,
            b,
            log_a,
            log_b,
            behavior,
        }
    }

    async fn wait_until(what: &str, condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    /// Wait until `log` has `n` entries for `command`.
    async fn wait_for_logged(log: &Log, command: &str, n: usize) {
        let what = format!("{command} to be sent {n} times");
        wait_until(&what, || {
            logged(log).iter().filter(|c| *c == command).count() >= n
        })
        .await;
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
        let Fixture {
            household,
            a,
            b,
            log_a,
            log_b,
            ..
        } = fixture().await;

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
        // Reuses the connection to B.
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
        assert_eq!((a.accepted(), b.accepted()), (1, 1));
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

    #[tokio::test]
    async fn restores_subscriptions_after_losing_a_connection() {
        let Fixture {
            household,
            b,
            log_b,
            ..
        } = fixture().await;
        let subscription = Subscription::GroupVolume(GroupId::new("B:1"));
        household.subscribe(&subscription).await.unwrap();
        let mut events = household.events();

        b.drop_connections();
        wait_for_logged(&log_b, "groupVolume:subscribe:B:1", 2).await;
        assert_eq!(b.accepted(), 2);
        // The player sends the current state again.
        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event.payload, EventPayload::GroupVolume(_)));
        assert!(household.subscriptions().contains(&subscription));
        // New handles use the new connection.
        household
            .group(&GroupId::new("B:1"))
            .await
            .unwrap()
            .set_volume(5)
            .await
            .unwrap();
        assert_eq!(b.accepted(), 2);
    }

    #[tokio::test]
    async fn restores_the_topology_after_losing_the_primary_connection() {
        let Fixture {
            household,
            a,
            log_a,
            ..
        } = fixture().await;
        household.subscribe(&Subscription::Favorites).await.unwrap();
        let mut updates = household.topology_updates();
        updates.mark_unchanged();

        a.drop_connections();
        wait_for_logged(&log_a, "groups:subscribe:Sonos_1", 2).await;
        wait_for_logged(&log_a, "favorites:subscribe:Sonos_1", 2).await;
        tokio::time::timeout(Duration::from_secs(1), updates.changed())
            .await
            .expect("topology not refreshed")
            .unwrap();
        household
            .connection()
            .await
            .unwrap()
            .get_groups()
            .await
            .unwrap();
        assert_eq!(a.accepted(), 2);
    }

    #[tokio::test]
    async fn keeps_retrying_until_the_player_is_back() {
        let Fixture {
            household,
            b,
            log_b,
            ..
        } = fixture().await;
        household
            .subscribe(&Subscription::PlayerVolume(PlayerId::new("B")))
            .await
            .unwrap();

        b.set_available(false);
        b.drop_connections();
        // Tried right away, then after 500ms.
        let start = tokio::time::Instant::now();
        wait_until("two attempts", || b.accepted() == 3).await;
        assert!(start.elapsed() >= Duration::from_millis(500));
        b.set_available(true);
        wait_for_logged(&log_b, "playerVolume:subscribe:B", 2).await;
        assert_eq!(b.accepted(), 4);
    }

    #[tokio::test]
    async fn does_not_reconnect_without_subscriptions() {
        let Fixture { household, b, .. } = fixture().await;
        let subscription = Subscription::GroupVolume(GroupId::new("B:1"));
        household.subscribe(&subscription).await.unwrap();
        household.unsubscribe(&subscription).await.unwrap();

        b.drop_connections();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(b.accepted(), 1);
        // Until it's needed.
        household.player(&PlayerId::new("B")).await.unwrap();
        assert_eq!(b.accepted(), 2);
    }

    #[tokio::test]
    async fn drops_subscriptions_to_groups_that_are_gone() {
        let Fixture { household, b, .. } = fixture().await;
        let subscription = Subscription::Playback(GroupId::new("B:1"));
        household.subscribe(&subscription).await.unwrap();
        // `B` joined `A:1` while the connection to it was being lost.
        let players = household.topology().players;
        household.inner.topology.send_modify(|topology| {
            topology.groups.remove(&GroupId::new("B:1"));
            topology
                .groups
                .get_mut(&GroupId::new("A:1"))
                .unwrap()
                .player_ids = players.iter().map(|p| p.id.clone()).collect();
        });

        b.drop_connections();
        wait_until("the subscription to be dropped", || {
            !household.subscriptions().contains(&subscription)
        })
        .await;
        assert_eq!(household.subscriptions(), [Subscription::Groups]);
    }

    #[tokio::test]
    async fn stops_reconnecting_once_closed() {
        let Fixture {
            household, a, b, ..
        } = fixture().await;
        household
            .subscribe(&Subscription::PlayerVolume(PlayerId::new("B")))
            .await
            .unwrap();
        household.close().await;
        assert!(household.subscriptions().is_empty());

        a.drop_connections();
        b.drop_connections();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!((a.accepted(), b.accepted()), (1, 1));
        let err = household.group(&GroupId::new("A:1")).await.unwrap_err();
        assert!(matches!(err, Error::ConnectionClosed), "{err:?}");
        let err = household.connection().await.unwrap_err();
        assert!(matches!(err, Error::ConnectionClosed), "{err:?}");
    }

    #[tokio::test]
    async fn stops_reconnecting_once_dropped() {
        let Fixture { household, b, .. } = fixture().await;
        household
            .subscribe(&Subscription::PlayerVolume(PlayerId::new("B")))
            .await
            .unwrap();
        b.set_available(false);
        b.drop_connections();
        wait_until("the first attempt", || b.accepted() == 2).await;

        drop(household);
        // Past the next attempt.
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(b.accepted(), 2);
    }

    #[tokio::test]
    async fn keeps_restoring_players_missing_from_the_topology() {
        let Fixture {
            household,
            b,
            log_b,
            ..
        } = fixture().await;
        let player_volume = Subscription::PlayerVolume(PlayerId::new("B"));
        let group_volume = Subscription::GroupVolume(GroupId::new("B:1"));
        household.subscribe(&player_volume).await.unwrap();
        household.subscribe(&group_volume).await.unwrap();

        // `B` restarts, and leaves the topology meanwhile.
        let topology = household.topology();
        b.set_available(false);
        household.inner.topology.send_modify(|topology| {
            topology.players.remove(&PlayerId::new("B"));
            topology.groups.remove(&GroupId::new("B:1"));
        });
        b.drop_connections();
        // Past the first retry.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(b.accepted(), 1);
        let subscriptions = household.subscriptions();
        assert!(subscriptions.contains(&player_volume));
        assert!(subscriptions.contains(&group_volume));

        // `B` is back.
        b.set_available(true);
        household.inner.topology.send_replace(topology);
        wait_for_logged(&log_b, "playerVolume:subscribe:B", 2).await;
        wait_for_logged(&log_b, "groupVolume:subscribe:B:1", 2).await;
    }

    #[tokio::test]
    async fn routes_restored_subscriptions_with_the_refreshed_topology() {
        let Fixture {
            household,
            a,
            log_a,
            log_b,
            behavior,
            ..
        } = fixture().await;
        let subscription = Subscription::GroupVolume(GroupId::new("A:1"));
        household.subscribe(&subscription).await.unwrap();

        // While the connection to `A` is lost, `B` becomes the coordinator of `A:1`.
        let players = household.topology().players;
        let url = |id: &str| {
            players
                .get(&PlayerId::new(id))
                .unwrap()
                .websocket_url
                .clone()
        };
        *behavior.groups_event.lock().unwrap() = json!({
            "_objectType": "groups",
            "groups": [group("A:1", "Kitchen", "B"), group("B:1", "Office", "B")],
            "players": [player("A", &url("A")), player("B", &url("B"))],
        });
        a.drop_connections();

        wait_for_logged(&log_b, "groupVolume:subscribe:A:1", 1).await;
        let sent_to_a = logged(&log_a)
            .iter()
            .filter(|c| *c == "groupVolume:subscribe:A:1")
            .count();
        assert_eq!(sent_to_a, 1);
    }

    #[tokio::test]
    async fn tries_unreachable_players_once_per_attempt() {
        let Fixture { household, b, .. } = fixture().await;
        for subscription in [
            Subscription::PlayerVolume(PlayerId::new("B")),
            Subscription::GroupVolume(GroupId::new("B:1")),
            Subscription::Playback(GroupId::new("B:1")),
        ] {
            household.subscribe(&subscription).await.unwrap();
        }

        b.set_available(false);
        b.drop_connections();
        wait_until("the first attempt", || b.accepted() == 2).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(b.accepted(), 2);
    }

    #[tokio::test]
    async fn drops_subscriptions_the_player_rejects() {
        let Fixture {
            household,
            b,
            behavior,
            ..
        } = fixture().await;
        let subscription = Subscription::PlayerVolume(PlayerId::new("B"));
        household.subscribe(&subscription).await.unwrap();

        // E.g. after a firmware update.
        behavior
            .rejected
            .lock()
            .unwrap()
            .insert("playerVolume:subscribe:B".to_owned());
        b.drop_connections();
        wait_until("the subscription to be dropped", || {
            !household.subscriptions().contains(&subscription)
        })
        .await;
        assert_eq!(b.accepted(), 2);
    }

    #[tokio::test]
    async fn follows_groups_to_their_new_coordinator() {
        let Fixture {
            household,
            b,
            log_a,
            log_b,
            behavior,
            ..
        } = fixture().await;
        let subscriptions = [
            Subscription::GroupVolume(GroupId::new("B:1")),
            Subscription::Playback(GroupId::new("B:1")),
        ];
        for subscription in &subscriptions {
            household.subscribe(subscription).await.unwrap();
        }

        // `A` now coordinates `B:1`, but the topology doesn't know yet.
        let url_a = household.inner.primary_url.clone();
        behavior
            .moves
            .lock()
            .unwrap()
            .insert("B:1".to_owned(), url_a.clone());
        let event = coordinator_changed(GroupStatus::Moved, Some(&url_a));
        // Twice, as when it comes from several connections.
        b.push(event.clone());
        b.push(event);

        wait_for_logged(&log_a, "groupVolume:subscribe:B:1", 1).await;
        wait_for_logged(&log_a, "playback:subscribe:B:1", 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Restored once each, through `B` which redirected to `A`.
        let log_a = logged(&log_a);
        assert_eq!(log_a.iter().filter(|c| c.contains(":B:1")).count(), 2);
        assert_eq!(
            logged(&log_b)
                .iter()
                .filter(|c| *c == "groupVolume:subscribe:B:1")
                .count(),
            2
        );
        for subscription in &subscriptions {
            assert!(household.subscriptions().contains(subscription));
        }

        // Nothing is left on `B` to restore.
        b.drop_connections();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(b.accepted(), 1);
    }

    #[tokio::test]
    async fn drops_subscriptions_when_a_group_is_gone() {
        let Fixture { household, b, .. } = fixture().await;
        let group_volume = Subscription::GroupVolume(GroupId::new("B:1"));
        let player_volume = Subscription::PlayerVolume(PlayerId::new("B"));
        household.subscribe(&group_volume).await.unwrap();
        household.subscribe(&player_volume).await.unwrap();
        let mut events = household.events();

        b.push(coordinator_changed(GroupStatus::Gone, None));
        wait_until("the subscription to be dropped", || {
            !household.subscriptions().contains(&group_volume)
        })
        .await;
        assert!(household.subscriptions().contains(&player_volume));
        // The event is still delivered.
        let event = events.recv().await.unwrap();
        assert!(matches!(
            event.payload,
            EventPayload::GroupCoordinatorChanged(_)
        ));
    }

    #[tokio::test]
    async fn keeps_subscriptions_when_a_group_is_updated() {
        let Fixture {
            household,
            b,
            log_a,
            log_b,
            behavior,
            ..
        } = fixture().await;
        let subscription = Subscription::GroupVolume(GroupId::new("B:1"));
        household.subscribe(&subscription).await.unwrap();

        // E.g. a player joined the group.
        b.push(coordinator_changed(GroupStatus::Updated, None));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(household.subscriptions().contains(&subscription));
        assert_eq!(logged(&log_b), ["groupVolume:subscribe:B:1"]);

        // A later move is still followed.
        let url_a = household.inner.primary_url.clone();
        behavior
            .moves
            .lock()
            .unwrap()
            .insert("B:1".to_owned(), url_a.clone());
        b.push(coordinator_changed(GroupStatus::Moved, Some(&url_a)));
        wait_for_logged(&log_a, "groupVolume:subscribe:B:1", 1).await;
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
