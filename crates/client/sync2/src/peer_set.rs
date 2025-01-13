use mc_p2p::{P2pCommands, PeerId};
use std::cmp;
use std::collections::{hash_map, BTreeSet, HashMap, HashSet};
use std::num::Saturating;
use tokio::time::{Duration, Instant};

// TODO: add bandwidth metric
#[derive(Default)]
struct PeerStats {
    // prioritize peers that are behaving correctly
    successes: Saturating<i32>,
    // avoid peers that are behaving incorrectly
    // TODO: we may want to differenciate timeout failures and bad-data failures.
    // => we probably want to evict bad-data failures in every case.
    failures: Saturating<i32>,
    // avoid peers that are currently in use
    in_use_counter: Saturating<u32>,
}

impl PeerStats {
    fn increment_successes(&mut self) {
        self.successes += 1;
    }
    fn increment_failures(&mut self) {
        self.failures += 1;
    }
    fn increment_in_use(&mut self) {
        self.in_use_counter += 1;
    }
    fn decrement_in_use(&mut self) {
        self.in_use_counter -= 1;
    }
    fn score(&self) -> i64 {
        // it's okay to use peers that are currently in use, but we don't want to rely on only one peer all the time
        // so, we put a temporary small malus if the peer is already in use.
        // if we are using the peer a lot, we want that malus to be higher - we really don't want to spam a single peer.
        let in_use_malus = if self.in_use_counter < Saturating(16) {
            self.in_use_counter
        } else {
            self.in_use_counter * Saturating(10)
        };
        let in_use_malus = Saturating(in_use_malus.0 as i32);

        // we only count up to 10 successes, to avoid having a score go too high.
        let successes = self.successes.max(Saturating(5));

        (Saturating(-10) * self.failures + successes - in_use_malus).0.into()
    }

    fn should_evict(&self) -> bool {
        self.failures >= Saturating(5)
    }
}

#[derive(Eq, PartialEq)]
struct PeerSortedByScore {
    peer_id: PeerId,
    score: i64,
}
impl PeerSortedByScore {
    pub fn new(peer_id: PeerId, stats: &PeerStats) -> Self {
        Self { peer_id, score: stats.score() }
    }
}

impl Ord for PeerSortedByScore {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.score.cmp(&other.score)
    }
}
impl PartialOrd for PeerSortedByScore {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Invariants:
// 1) there should be a one-to-one correspondance between the `queue` and `stats_by_peer` variable.
#[derive(Default)]
struct PeerSetInner {
    queue: BTreeSet<PeerSortedByScore>,
    stats_by_peer: HashMap<PeerId, PeerStats>,
}

impl PeerSetInner {
    fn peek_next(&self) -> Option<PeerId> {
        self.queue.first().map(|p| p.peer_id)
    }

    fn update_stats(&mut self, peer: PeerId, f: impl FnOnce(&mut PeerStats)) {
        match self.stats_by_peer.entry(peer) {
            hash_map::Entry::Occupied(mut entry) => {
                // Remove old queue entry.
                debug_assert!(self.queue.remove(&PeerSortedByScore::new(peer, entry.get())), "Invariant 1 violated");

                // Update the stats in-place
                f(entry.get_mut());

                if entry.get().should_evict() {
                    // evict
                    entry.remove();
                } else {
                    // Reinsert the queue entry with the new score.
                    // If insert returns true, the value is already in the queue - which would mean that the peer id is duplicated in the queue.
                    // `stats_by_peer` has PeerId as key and as such cannot have a duplicate peer id. This means that if there is a duplicated
                    // peer_id in the queue, there is not a one-to-one correspondance between the two datastructures.
                    debug_assert!(self.queue.insert(PeerSortedByScore::new(peer, entry.get())), "Invariant 1 violated");
                }
            }
            hash_map::Entry::Vacant(_entry) => {}
        }
    }

    fn append_new_peers(&mut self, new_peers: impl IntoIterator<Item = PeerId>) {
        for peer_id in new_peers.into_iter() {
            if let hash_map::Entry::Vacant(entry) = self.stats_by_peer.entry(peer_id) {
                let stats = PeerStats::default();
                self.queue.insert(PeerSortedByScore::new(peer_id, &stats));
                entry.insert(stats);
            }
        }
    }
}

pub struct GetPeersInner {
    wait_until: Option<Instant>,
    commands: P2pCommands,
}

impl GetPeersInner {
    /// We avoid spamming get_random_peers: the start of each get_random_peers request must be separated by at least this duration.
    /// This has no effect if the get_random_peers operation takes more time to complete than this delay.
    const GET_RANDOM_PEERS_DELAY: Duration = Duration::from_millis(100);

    pub fn new(commands: P2pCommands) -> Self {
        Self { commands, wait_until: None }
    }

    pub async fn get_new_peers(&mut self) -> HashSet<PeerId> {
        let now = Instant::now();

        if let Some(inst) = self.wait_until {
            if inst > now {
                tokio::time::sleep_until(inst).await;
            }
        }
        self.wait_until = Some(now + Self::GET_RANDOM_PEERS_DELAY);

        let mut res = self.commands.get_random_peers().await;
        tracing::debug!("Got get_random_peers answer: {res:?}");
        res.remove(&self.commands.peer_id()); // remove ourselves from the response, in case we find ourselves
        res
    }
}

// TODO: eviction ban list? if we evicted a peer from the set, we may want to block it for some delay.
// TODO: we may want to invalidate the peer list over time
// Mutex order: to statically ensure deadlocks are not possible, inner should always be locked after get_peers_mutex, if the two need to be taken at once.
pub struct PeerSet {
    // Tokio mutex: when the peer set is empty, we want to .await to get new peers
    // This is behind a mutex because we don't want to have concurrent get_more_peers requests. If there is already a request in flight, this mutex ensures we wait until that
    // request finishes before trying to get even more peers.
    get_more_peers_mutex: tokio::sync::Mutex<GetPeersInner>,
    // Std mutex: underlying datastructure, all accesses are sync
    inner: std::sync::Mutex<PeerSetInner>,
}

impl PeerSet {
    pub fn new(commands: P2pCommands) -> Self {
        Self {
            get_more_peers_mutex: tokio::sync::Mutex::new(GetPeersInner::new(commands)),
            inner: std::sync::Mutex::new(PeerSetInner::default()),
        }
    }

    /// Returns the next peer to use. If there is are no peers currently in the set,
    /// it will start a get random peers command.
    // TODO: keep track of the current number of request per peer, and avoid over-using a single peer.
    // TODO: if we really have to use a peer that just had a peer operation error, delay the next peer request a little
    // bit so that we don't spam that peer with requests.
    pub async fn next_peer(&self) -> anyhow::Result<PeerId> {
        fn next_from_set(inner: &mut PeerSetInner) -> Option<PeerId> {
            inner.peek_next().inspect(|peer| {
                // this will update the queue order, so that we can return another peer next time this function is called.
                inner.update_stats(*peer, |stats| {
                    stats.increment_in_use();
                });
            })
        }

        if let Some(peer) = next_from_set(&mut self.inner.lock().expect("Poisoned lock")) {
            return Ok(peer);
        }

        loop {
            let mut guard = self.get_more_peers_mutex.lock().await;

            // Some other task may have filled the peer set for us while we were waiting.
            if let Some(peer) = next_from_set(&mut self.inner.lock().expect("Poisoned lock")) {
                return Ok(peer);
            }

            let new_peers = guard.get_new_peers().await;
            // note: this is the only place where the two locks are taken at the same time.
            // see structure detail for lock order.
            let mut inner = self.inner.lock().expect("Poisoned lock");
            inner.append_new_peers(new_peers);

            if let Some(peer) = next_from_set(&mut inner) {
                return Ok(peer);
            }
        }
    }

    /// Signal that the peer did not follow the protocol correctly, sent bad data or timed out.
    /// We may want to avoid this peer in the future.
    pub fn peer_operation_error(&self, peer_id: PeerId) {
        tracing::debug!("peer_operation_error: {peer_id:?}");
        let mut inner = self.inner.lock().expect("Poisoned lock");
        inner.update_stats(peer_id, |stats| {
            stats.decrement_in_use();
            stats.increment_failures();
        })
    }

    /// Signal that the operation with the peer was successful.
    ///
    // TODO: add a bandwidth argument to allow the peer set to score and avoid being drip-fed.
    pub fn peer_operation_success(&self, peer_id: PeerId) {
        tracing::debug!("peer_operation_success: {peer_id:?}");
        let mut inner = self.inner.lock().expect("Poisoned lock");
        inner.update_stats(peer_id, |stats| {
            stats.decrement_in_use();
            stats.increment_successes();
        })
    }
}
