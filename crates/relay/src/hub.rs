//! The sync sockets: one per connected machine, grouped by identity. A socket carries signals
//! and no data: `blobs` when the identity has a blob this machine may read, `machines` when
//! its machine list or their presence changed. A machine is online while it has a socket here.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    Blobs,
    Machines,
}

impl Signal {
    pub fn json(self) -> &'static str {
        match self {
            Signal::Blobs => r#"{"type":"blobs"}"#,
            Signal::Machines => r#"{"type":"machines"}"#,
        }
    }
}

struct Connection {
    id: u64,
    machine_pubkey: String,
    signals: mpsc::Sender<Signal>,
}

#[derive(Default)]
pub struct Hub {
    identities: Mutex<HashMap<String, Vec<Connection>>>,
    next_id: AtomicU64,
}

/// A socket's seat in the hub.
pub struct Seat {
    pub id: u64,
    pub signals: mpsc::Receiver<Signal>,
    /// The machine had no other socket: it just came online.
    pub came_online: bool,
}

impl Hub {
    pub fn join(&self, identity_pubkey: &str, machine_pubkey: &str) -> Seat {
        // A signal says "look again", so a few are as good as many: a full queue drops them.
        let (signals, receiver) = mpsc::channel(4);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut identities = self.lock();
        let connections = identities.entry(identity_pubkey.to_string()).or_default();
        let came_online = !connections.iter().any(|c| c.machine_pubkey == machine_pubkey);
        connections.push(Connection { id, machine_pubkey: machine_pubkey.to_string(), signals });
        Seat { id, signals: receiver, came_online }
    }

    /// True when that was the machine's last socket: it went offline.
    pub fn leave(&self, identity_pubkey: &str, id: u64) -> bool {
        let mut identities = self.lock();
        let Some(connections) = identities.get_mut(identity_pubkey) else { return false };
        let Some(index) = connections.iter().position(|c| c.id == id) else { return false };
        let left = connections.swap_remove(index);
        let went_offline = !connections.iter().any(|c| c.machine_pubkey == left.machine_pubkey);
        if connections.is_empty() {
            identities.remove(identity_pubkey);
        }
        went_offline
    }

    /// A blob landed. One sealed to a machine concerns that machine alone.
    pub fn blobs(&self, identity_pubkey: &str, recipient_machine_pubkey: Option<&str>) {
        self.signal(identity_pubkey, Signal::Blobs, |c| recipient_machine_pubkey.is_none_or(|m| m == c.machine_pubkey));
    }

    pub fn machines(&self, identity_pubkey: &str) {
        self.signal(identity_pubkey, Signal::Machines, |_| true);
    }

    /// Every socket here looks again: this process may have missed events.
    pub fn everyone(&self) {
        for connections in self.lock().values() {
            for connection in connections {
                let _ = connection.signals.try_send(Signal::Blobs);
                let _ = connection.signals.try_send(Signal::Machines);
            }
        }
    }

    /// Closes a machine's sockets: their senders go, and each socket task ends on that.
    pub fn kick(&self, identity_pubkey: &str, machine_pubkey: &str) {
        if let Some(connections) = self.lock().get_mut(identity_pubkey) {
            connections.retain(|c| c.machine_pubkey != machine_pubkey);
        }
    }

    pub fn online(&self, identity_pubkey: &str) -> HashSet<String> {
        self.lock().get(identity_pubkey).map(|connections| connections.iter().map(|c| c.machine_pubkey.clone()).collect()).unwrap_or_default()
    }

    fn signal(&self, identity_pubkey: &str, signal: Signal, to: impl Fn(&Connection) -> bool) {
        if let Some(connections) = self.lock().get(identity_pubkey) {
            for connection in connections.iter().filter(|c| to(c)) {
                let _ = connection.signals.try_send(signal);
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Connection>>> {
        self.identities.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_follows_the_sockets() {
        let hub = Hub::default();
        let mac = hub.join("identity", "mac");
        assert!(mac.came_online);
        let second = hub.join("identity", "mac");
        assert!(!second.came_online);
        assert!(!hub.leave("identity", second.id));
        assert_eq!(hub.online("identity"), HashSet::from(["mac".to_string()]));
        assert!(hub.leave("identity", mac.id));
        assert!(hub.online("identity").is_empty());
    }

    #[test]
    fn an_envelope_signals_its_recipient_alone() {
        let hub = Hub::default();
        let mut mac = hub.join("identity", "mac");
        let mut phone = hub.join("identity", "phone");
        let mut stranger = hub.join("other", "mac");
        hub.blobs("identity", Some("mac"));
        assert_eq!(mac.signals.try_recv().ok(), Some(Signal::Blobs));
        assert!(phone.signals.try_recv().is_err());
        hub.blobs("identity", None);
        assert_eq!(phone.signals.try_recv().ok(), Some(Signal::Blobs));
        assert!(stranger.signals.try_recv().is_err());
    }

    #[test]
    fn a_kicked_machine_loses_its_sockets() {
        let hub = Hub::default();
        let mut mac = hub.join("identity", "mac");
        hub.kick("identity", "mac");
        assert!(matches!(mac.signals.try_recv(), Err(mpsc::error::TryRecvError::Disconnected)));
        assert!(!hub.leave("identity", mac.id));
    }
}
