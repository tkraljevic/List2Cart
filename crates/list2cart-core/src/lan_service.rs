//! LAN discovery and automatic local sync coordinator (`S10`).
//! Combines `syncnet::transport::{discovery, lan}` with `session.rs`
//! and `Database` without holding `Mutex<Database>` across `.await`.

use crate::{
    apply::PROTOCOL_NAME,
    db::Database,
    session::{
        SessionConfig, SessionOutcome, dial_and_run_conversation_on, run_conversation_until_idle,
    },
};
use chrono::Utc;
pub use syncnet::transport::discovery::{FoundPeer, clear_pairing_flag, write_pairing_flag};
use syncnet::{
    identity::{parse_public_key, public_key_to_base64},
    tls::{CredentialsProvider, TlsAuthError, accept_exchange_on},
    transport::{
        discovery::{
            Beacon, DEFAULT_DISCOVERY_PORT, announce_on, discover_lan, discover_on,
            read_pairing_flag,
        },
        lan::{
            ActivePeers, CoalescedTrigger, DEFAULT_CONNECT_TIMEOUT, DEFAULT_DEBOUNCE_WINDOW,
            DEFAULT_MAX_CONCURRENT_CONVERSATIONS, DEFAULT_MAX_CONCURRENT_HANDSHAKES,
            DEFAULT_PERIODIC_INTERVAL, TriggerWake, connect_tcp_with_timeout, serve_tcp_listener,
        },
    },
    wire::conversation::{ConversationEnvelope, ConversationError, RefusalCode, send_envelope},
};
use tokio::{
    self,
    net::{TcpListener, TcpStream, UdpSocket},
    sync::Mutex as AsyncMutex,
    task::JoinHandle,
};

/// Discovers List2Cart peers on the local LAN using the default discovery port and protocol.
pub async fn discover_lan_peers(timeout: Duration) -> Result<Vec<FoundPeer>, String> {
    discover_lan(DEFAULT_DISCOVERY_PORT, PROTOCOL_NAME, timeout, &[])
        .await
        .map_err(|e| format!("Greska LAN otkrivanja: {e}"))
}
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct LanServiceConfig {
    pub protocol: String,
    pub device_name: String,
    pub exchange_bind_addr: SocketAddr,
    /// `None` disables the UDP discovery announcer/prober (manual address mode only).
    pub discovery_bind_addr: Option<SocketAddr>,
    /// Optional explicit UDP targets for discovery probes (used in tests or non-broadcast LANs).
    /// When `None`, uses LAN subnet broadcast on `discovery_port`.
    pub discovery_targets: Option<Vec<SocketAddr>>,
    pub discovery_port: u16,
    pub discovery_timeout: Duration,
    pub connect_timeout: Duration,
    pub debounce: Duration,
    pub periodic_interval: Duration,
    pub max_concurrent_handshakes: usize,
    pub max_concurrent_conversations: usize,
    pub data_dir: Option<PathBuf>,
    pub session: SessionConfig,
}

impl Default for LanServiceConfig {
    fn default() -> Self {
        Self {
            protocol: PROTOCOL_NAME.to_owned(),
            device_name: "List2Cart".to_owned(),
            exchange_bind_addr: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            discovery_bind_addr: Some(SocketAddr::from((
                Ipv4Addr::UNSPECIFIED,
                DEFAULT_DISCOVERY_PORT,
            ))),
            discovery_targets: None,
            discovery_port: DEFAULT_DISCOVERY_PORT,
            discovery_timeout: Duration::from_millis(700),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            debounce: DEFAULT_DEBOUNCE_WINDOW,
            periodic_interval: DEFAULT_PERIODIC_INTERVAL,
            max_concurrent_handshakes: DEFAULT_MAX_CONCURRENT_HANDSHAKES,
            max_concurrent_conversations: DEFAULT_MAX_CONCURRENT_CONVERSATIONS,
            data_dir: None,
            session: SessionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerSyncRecord {
    pub peer_public_key: String,
    pub peer_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_path: Option<String>,
    pub last_applied: usize,
    pub last_sent: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanServiceStatus {
    pub running: bool,
    #[serde(default)]
    pub paused: bool,
    pub exchange_addr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_addr: Option<String>,
    pub device_public_key: String,
    pub active_conversations: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_round_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub peers: Vec<PeerSyncRecord>,
}

#[derive(Debug, Default)]
struct SharedState {
    manual_addrs: HashMap<String, SocketAddr>,
    peers: BTreeMap<String, PeerSyncRecord>,
    last_round_at: Option<String>,
    last_error: Option<String>,
}

pub struct LanSyncService {
    db_mutex: Arc<Mutex<Database>>,
    cfg: LanServiceConfig,
    device_pub_b64: String,
    exchange_addr: SocketAddr,
    discovery_addr: Option<SocketAddr>,
    active_peers: ActivePeers,
    trigger: CoalescedTrigger,
    paused: Arc<AtomicBool>,
    state: Arc<Mutex<SharedState>>,
    sync_lock: Arc<AsyncMutex<()>>,
    cancel: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

impl LanSyncService {
    pub async fn start(
        db_mutex: Arc<Mutex<Database>>,
        cfg: LanServiceConfig,
    ) -> Result<Self, String> {
        let device_pub_b64 = {
            let db = db_mutex
                .lock()
                .map_err(|_| "Greska zakljucavanja baze".to_owned())?;
            db.sync_identity().device_public_key
        };

        let tcp_listener = TcpListener::bind(cfg.exchange_bind_addr)
            .await
            .map_err(|e| format!("Greska pokretanja LAN slusalice: {e}"))?;
        let exchange_addr = tcp_listener
            .local_addr()
            .map_err(|e| format!("Greska adrese LAN slusalice: {e}"))?;

        let (udp_socket, discovery_addr) = match cfg.discovery_bind_addr {
            Some(bind_addr) => {
                let sock = UdpSocket::bind(bind_addr)
                    .await
                    .map_err(|e| format!("Greska pokretanja LAN otkrivanja: {e}"))?;
                let _ = sock.set_broadcast(true);
                let local = sock
                    .local_addr()
                    .map_err(|e| format!("Greska adrese LAN otkrivanja: {e}"))?;
                (Some(Arc::new(sock)), Some(local))
            }
            None => (None, None),
        };

        let active_peers = ActivePeers::new();
        let trigger = CoalescedTrigger::new(cfg.debounce, cfg.periodic_interval);
        let paused = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(SharedState::default()));
        let sync_lock = Arc::new(AsyncMutex::new(()));
        let cancel = CancellationToken::new();
        let mut tasks = Vec::new();

        // 1. Authenticated LAN TLS 1.3 listener
        {
            let db_for_accept = Arc::clone(&db_mutex);
            let active_for_accept = active_peers.clone();
            let state_for_accept = Arc::clone(&state);
            let session_cfg = cfg.session.clone();
            let cancel_accept = cancel.clone();
            let max_conc = cfg
                .max_concurrent_handshakes
                .max(cfg.max_concurrent_conversations);
            tasks.push(tokio::spawn(async move {
                serve_tcp_listener(
                    tcp_listener,
                    cancel_accept,
                    max_conc,
                    move |stream, addr| {
                        let db_for_accept = Arc::clone(&db_for_accept);
                        let active_for_accept = active_for_accept.clone();
                        let state_for_accept = Arc::clone(&state_for_accept);
                        let session_cfg = session_cfg.clone();
                        async move {
                            handle_incoming_exchange(
                                stream,
                                addr,
                                db_for_accept,
                                active_for_accept,
                                state_for_accept,
                                session_cfg,
                            )
                            .await;
                        }
                    },
                )
                .await;
            }));
        }

        // 2. UDP LAN discovery announcer
        if let Some(sock) = udp_socket {
            let cancel_ann = cancel.clone();
            let protocol = cfg.protocol.clone();
            let device_id = device_pub_b64.clone();
            let name = cfg.device_name.clone();
            let ex_port = exchange_addr.port();
            let data_dir = cfg.data_dir.clone();
            let info: Arc<dyn Fn() -> Beacon + Send + Sync> = Arc::new(move || Beacon {
                protocol: protocol.clone(),
                device_id: device_id.clone(),
                name: name.clone(),
                exchange_port: ex_port,
                pair_port: data_dir.as_deref().and_then(read_pairing_flag),
                meta: BTreeMap::new(),
            });
            let proto_ann = cfg.protocol.clone();
            tasks.push(tokio::spawn(async move {
                let _ = announce_on(sock, cancel_ann, proto_ann, info).await;
            }));
        }

        // 3. Coalesced local-change + periodic safety-net sync coordinator
        {
            let db_for_loop = Arc::clone(&db_mutex);
            let cfg_for_loop = cfg.clone();
            let dev_pub_for_loop = device_pub_b64.clone();
            let active_for_loop = active_peers.clone();
            let trigger_for_loop = trigger.clone();
            let paused_for_loop = Arc::clone(&paused);
            let state_for_loop = Arc::clone(&state);
            let sync_lock_for_loop = Arc::clone(&sync_lock);
            let cancel_for_loop = cancel.clone();
            tasks.push(tokio::spawn(async move {
                loop {
                    match trigger_for_loop.next_wake(&cancel_for_loop).await {
                        TriggerWake::Cancelled => break,
                        TriggerWake::LocalChange | TriggerWake::Periodic => {
                            if paused_for_loop.load(Ordering::Relaxed) {
                                continue;
                            }
                            let _guard = sync_lock_for_loop.lock().await;
                            let _ = run_discovery_and_sync_pass(
                                &db_for_loop,
                                &cfg_for_loop,
                                &dev_pub_for_loop,
                                &active_for_loop,
                                &state_for_loop,
                            )
                            .await;
                        }
                    }
                }
            }));
        }

        Ok(Self {
            db_mutex,
            cfg,
            device_pub_b64,
            exchange_addr,
            discovery_addr,
            active_peers,
            trigger,
            paused,
            state,
            sync_lock,
            cancel,
            tasks,
        })
    }

    #[must_use]
    pub fn exchange_addr(&self) -> SocketAddr {
        self.exchange_addr
    }

    #[must_use]
    pub fn discovery_addr(&self) -> Option<SocketAddr> {
        self.discovery_addr
    }

    #[must_use]
    pub fn device_public_key(&self) -> &str {
        &self.device_pub_b64
    }

    /// Signals that local data changed; coalesced through the debounce window before syncing.
    pub fn notify_local_change(&self) {
        if !self.is_paused() {
            self.trigger.notify_change();
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
        if !paused {
            self.trigger.notify_change();
        }
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Cancels all background LAN listener and sync tasks.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn set_manual_peer_addr(&self, peer_public_key: &str, addr: Option<SocketAddr>) {
        if let Ok(mut st) = self.state.lock() {
            match addr {
                Some(a) => {
                    st.manual_addrs.insert(peer_public_key.to_owned(), a);
                }
                None => {
                    st.manual_addrs.remove(peer_public_key);
                }
            }
        }
    }

    /// Runs an immediate LAN discovery + manual-address sync pass across known peers.
    pub async fn sync_now(&self) -> Result<usize, String> {
        let _guard = self.sync_lock.lock().await;
        run_discovery_and_sync_pass(
            &self.db_mutex,
            &self.cfg,
            &self.device_pub_b64,
            &self.active_peers,
            &self.state,
        )
        .await
    }

    /// Directly dials and syncs with a specific known peer at `addr` (manual fallback).
    pub async fn sync_peer_at(
        &self,
        peer_public_key: &str,
        addr: SocketAddr,
    ) -> Result<SessionOutcome, String> {
        let expected_key = parse_public_key(peer_public_key)
            .map_err(|e| format!("Neispravan kljuc partnera: {e}"))?;
        let key_bytes = *expected_key.as_bytes();
        let Some(_flight) = self.active_peers.try_acquire(key_bytes) else {
            return Err("Razgovor s ovim partnerom je vec u tijeku".to_owned());
        };
        dial_peer_and_record(
            &self.db_mutex,
            &self.cfg,
            &self.state,
            peer_public_key,
            key_bytes,
            addr,
            "manual",
        )
        .await
    }

    #[must_use]
    pub fn status(&self) -> LanServiceStatus {
        let known_members = self
            .db_mutex
            .lock()
            .ok()
            .and_then(|db| db.list_members().ok())
            .unwrap_or_default();
        let st = self.state.lock().ok();

        let mut peers = Vec::new();
        for m in known_members {
            if m.peer_id == self.device_pub_b64 {
                continue;
            }
            let existing = st.as_ref().and_then(|s| s.peers.get(&m.peer_id));
            let manual_addr = st
                .as_ref()
                .and_then(|s| s.manual_addrs.get(&m.peer_id))
                .map(|a| a.to_string());
            peers.push(PeerSyncRecord {
                peer_public_key: m.peer_id,
                peer_name: m.name,
                last_addr: existing.and_then(|e| e.last_addr.clone()),
                manual_addr,
                last_sync_at: existing.and_then(|e| e.last_sync_at.clone()),
                last_path: existing.and_then(|e| e.last_path.clone()),
                last_applied: existing.map_or(0, |e| e.last_applied),
                last_sent: existing.map_or(0, |e| e.last_sent),
                last_error: existing.and_then(|e| e.last_error.clone()),
            });
        }

        LanServiceStatus {
            running: !self.cancel.is_cancelled(),
            paused: self.is_paused(),
            exchange_addr: self.exchange_addr.to_string(),
            discovery_addr: self.discovery_addr.map(|a| a.to_string()),
            device_public_key: self.device_pub_b64.clone(),
            active_conversations: self.active_peers.len(),
            last_round_at: st.as_ref().and_then(|s| s.last_round_at.clone()),
            last_error: st.as_ref().and_then(|s| s.last_error.clone()),
            peers,
        }
    }

    pub async fn shutdown(self) {
        self.cancel.cancel();
        for handle in self.tasks {
            let _ = handle.await;
        }
    }
}

async fn handle_incoming_exchange(
    stream: TcpStream,
    remote_addr: SocketAddr,
    db_mutex: Arc<Mutex<Database>>,
    active_peers: ActivePeers,
    state: Arc<Mutex<SharedState>>,
    config: SessionConfig,
) {
    let (sk, creds) = {
        let Ok(db) = db_mutex.lock() else {
            return;
        };
        let Ok(creds) = db.tls_credentials() else {
            return;
        };
        (db.clone_device_signing_key(), creds)
    };
    let creds_fn: Option<CredentialsProvider> = Some(Arc::new(move || creds.clone()));
    let db_for_gate = Arc::clone(&db_mutex);

    let mut auth = match accept_exchange_on(
        stream,
        &sk,
        &config.protocol,
        creds_fn,
        move |pk, peer_creds| {
            db_for_gate
                .lock()
                .ok()
                .and_then(|db| db.verify_or_recover_peer_access(pk, peer_creds).ok())
                .unwrap_or(false)
        },
    )
    .await
    {
        Ok(auth) => auth,
        Err(TlsAuthError::UntrustedPeer { .. }) => return,
        Err(_) => return,
    };

    let peer_b64 = public_key_to_base64(&auth.peer_key);
    let Some(_flight) = active_peers.try_acquire(auth.peer_key) else {
        let env = ConversationEnvelope::refusal(
            RefusalCode::Busy,
            "Razgovor s ovim partnerom je vec u tijeku",
        );
        let _ = send_envelope(&mut auth.stream, &env, config.step_timeout).await;
        return;
    };

    let result = run_conversation_until_idle(
        &mut auth.stream,
        &db_mutex,
        &auth.peer_key,
        auth.peer_credentials.as_deref(),
        false,
        &config,
    )
    .await;

    record_peer_result(&state, &peer_b64, remote_addr, "incoming", result.as_ref());
}

async fn run_discovery_and_sync_pass(
    db_mutex: &Arc<Mutex<Database>>,
    cfg: &LanServiceConfig,
    self_pub_b64: &str,
    active_peers: &ActivePeers,
    state: &Arc<Mutex<SharedState>>,
) -> Result<usize, String> {
    let members = {
        let db = db_mutex
            .lock()
            .map_err(|_| "Greska zakljucavanja baze".to_owned())?;
        db.list_members()?
    };

    let wanted_ids: Vec<String> = members
        .iter()
        .filter(|m| m.peer_id != self_pub_b64)
        .map(|m| m.peer_id.clone())
        .collect();

    if wanted_ids.is_empty() {
        return Ok(0);
    }

    let mut discovered: HashMap<String, SocketAddr> = HashMap::new();
    if cfg.discovery_bind_addr.is_some() {
        let found: Vec<FoundPeer> = match &cfg.discovery_targets {
            Some(targets) if !targets.is_empty() => {
                let sock = UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                    .await
                    .map_err(|e| format!("Greska UDP uticnice za otkrivanje: {e}"))?;
                discover_on(
                    &sock,
                    targets,
                    &cfg.protocol,
                    cfg.discovery_timeout,
                    &wanted_ids,
                )
                .await
                .unwrap_or_default()
            }
            _ => discover_lan(
                cfg.discovery_port,
                &cfg.protocol,
                cfg.discovery_timeout,
                &wanted_ids,
            )
            .await
            .unwrap_or_default(),
        };
        for peer in found {
            discovered.insert(peer.beacon.device_id.clone(), peer.exchange_addr());
        }
    }

    let manual_addrs = state
        .lock()
        .map(|s| s.manual_addrs.clone())
        .unwrap_or_default();

    let mut synced = 0usize;
    for peer_id in wanted_ids {
        let (addr, path_label) = if let Some(addr) = discovered.get(&peer_id).copied() {
            (addr, "lan")
        } else if let Some(addr) = manual_addrs.get(&peer_id).copied() {
            (addr, "manual")
        } else {
            continue;
        };

        let Ok(vk) = parse_public_key(&peer_id) else {
            continue;
        };
        let key_bytes = *vk.as_bytes();
        let Some(_flight) = active_peers.try_acquire(key_bytes) else {
            continue;
        };

        if dial_peer_and_record(db_mutex, cfg, state, &peer_id, key_bytes, addr, path_label)
            .await
            .is_ok()
        {
            synced += 1;
        }
    }

    if let Ok(mut st) = state.lock() {
        st.last_round_at = Some(Utc::now().to_rfc3339());
    }
    Ok(synced)
}

async fn dial_peer_and_record(
    db_mutex: &Mutex<Database>,
    cfg: &LanServiceConfig,
    state: &Mutex<SharedState>,
    peer_b64: &str,
    peer_key: [u8; 32],
    addr: SocketAddr,
    path_label: &str,
) -> Result<SessionOutcome, String> {
    let stream = match connect_tcp_with_timeout(addr, cfg.connect_timeout).await {
        Ok(s) => s,
        Err(e) => {
            let err = ConversationError::Io(e.to_string());
            record_peer_result(state, peer_b64, addr, path_label, Err(&err));
            return Err(err.to_string());
        }
    };

    let outcome =
        dial_and_run_conversation_on(stream, db_mutex, &addr.to_string(), &peer_key, &cfg.session)
            .await;

    record_peer_result(state, peer_b64, addr, path_label, outcome.as_ref());
    outcome.map_err(|e| e.to_string())
}

fn record_peer_result(
    state: &Mutex<SharedState>,
    peer_b64: &str,
    addr: SocketAddr,
    path_label: &str,
    result: Result<&SessionOutcome, &ConversationError>,
) {
    let Ok(mut st) = state.lock() else {
        return;
    };
    let entry = st
        .peers
        .entry(peer_b64.to_owned())
        .or_insert_with(|| PeerSyncRecord {
            peer_public_key: peer_b64.to_owned(),
            peer_name: String::new(),
            last_addr: None,
            manual_addr: None,
            last_sync_at: None,
            last_path: None,
            last_applied: 0,
            last_sent: 0,
            last_error: None,
        });
    entry.last_addr = Some(addr.to_string());
    entry.last_path = Some(path_label.to_owned());
    match result {
        Ok(outcome) => {
            entry.last_sync_at = Some(Utc::now().to_rfc3339());
            entry.last_applied = outcome.total_applied_versions;
            entry.last_sent = outcome.total_peer_applied_versions;
            entry.last_error = None;
            st.last_error = None;
        }
        Err(err) => {
            let msg = err.to_string();
            entry.last_error = Some(msg.clone());
            st.last_error = Some(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::PeerRole;
    use crate::pairing_session::PairingController;
    use ed25519_dalek::SigningKey;
    use syncnet::identity::generate_signing_key;
    use syncnet::tls::accept_exchange_on;
    use syncnet::transport::discovery::announce_on;

    fn temp_db_path(label: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("l2c-lan-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("list2cart.sqlite");
        (dir, db)
    }

    async fn pair_two_databases(
        db_a: Arc<Mutex<Database>>,
        db_b: Arc<Mutex<Database>>,
        role: PeerRole,
    ) {
        let ctrl_a = PairingController::default();
        let ctrl_b = PairingController::default();

        let (sk_a, id_a) = {
            let db = db_a.lock().unwrap();
            let id = PairingController::build_pair_identity(
                &db.sync_identity().device_public_key,
                "MacBook A",
                PROTOCOL_NAME,
            );
            (db.clone_device_signing_key(), id)
        };
        let (sk_b, id_b) = {
            let db = db_b.lock().unwrap();
            let id = PairingController::build_pair_identity(
                &db.sync_identity().device_public_key,
                "PC B",
                PROTOCOL_NAME,
            );
            (db.clone_device_signing_key(), id)
        };

        let listen_status = ctrl_b
            .start_listening(
                SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                sk_b,
                id_b,
                false,
            )
            .await
            .unwrap();
        let port = listen_status.listen_port.unwrap();
        let dial_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

        ctrl_a.dial(dial_addr, sk_a, id_a, true).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !ctrl_b.status(false).pending {
            assert!(
                tokio::time::Instant::now() < deadline,
                "ctrl_b nije stigao u pending stanje"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let (res_a, res_b) = tokio::join!(
            ctrl_a.confirm(&db_a, true, role, "Tjedna", true),
            ctrl_b.confirm(&db_b, true, role, "Tjedna", false)
        );
        assert!(res_a.unwrap().paired);
        assert!(res_b.unwrap().paired);
    }

    #[tokio::test]
    async fn two_paired_nodes_discover_over_lan_and_converge_on_local_change() {
        let (dir_a, path_a) = temp_db_path("a");
        let (dir_b, path_b) = temp_db_path("b");
        let db_a = Arc::new(Mutex::new(Database::open(&path_a).unwrap()));
        let db_b = Arc::new(Mutex::new(Database::open(&path_b).unwrap()));

        pair_two_databases(Arc::clone(&db_a), Arc::clone(&db_b), PeerRole::OwnDevice).await;

        let svc_b = LanSyncService::start(
            Arc::clone(&db_b),
            LanServiceConfig {
                device_name: "PC B".to_owned(),
                exchange_bind_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                discovery_bind_addr: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
                periodic_interval: Duration::from_secs(300),
                debounce: Duration::from_millis(40),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let b_disc = svc_b.discovery_addr().unwrap();

        let svc_a = LanSyncService::start(
            Arc::clone(&db_a),
            LanServiceConfig {
                device_name: "MacBook A".to_owned(),
                exchange_bind_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                discovery_bind_addr: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
                discovery_targets: Some(vec![b_disc]),
                periodic_interval: Duration::from_secs(300),
                debounce: Duration::from_millis(40),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        {
            let db = db_a.lock().unwrap();
            db.add_store("LAN Trgovina", "Split", "#1768D1").unwrap();
        }
        svc_a.notify_local_change();

        let mut converged = false;
        for _ in 0..40 {
            let has_store = db_b
                .lock()
                .unwrap()
                .list_stores()
                .unwrap()
                .iter()
                .any(|s| s.name == "LAN Trgovina");
            let status_recorded = svc_a
                .status()
                .peers
                .first()
                .and_then(|p| p.last_path.as_deref())
                == Some("lan");
            if has_store && status_recorded {
                converged = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(converged, "Peer B did not receive LAN Trgovina");

        let status_a = svc_a.status();
        assert_eq!(status_a.peers.len(), 1);
        assert_eq!(status_a.peers[0].last_path.as_deref(), Some("lan"));
        assert!(status_a.peers[0].last_error.is_none());

        svc_a.shutdown().await;
        svc_b.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[tokio::test]
    async fn fake_beacon_with_wrong_tls_key_is_rejected() {
        let (dir_a, path_a) = temp_db_path("fake-a");
        let (dir_b, path_b) = temp_db_path("fake-b");
        let db_a = Arc::new(Mutex::new(Database::open(&path_a).unwrap()));
        let db_b = Arc::new(Mutex::new(Database::open(&path_b).unwrap()));

        pair_two_databases(Arc::clone(&db_a), Arc::clone(&db_b), PeerRole::OwnDevice).await;
        let b_pub = db_b.lock().unwrap().sync_identity().device_public_key;

        // Start rogue TLS server with a different SigningKey and rogue UDP beacon claiming B's device_id
        let rogue_sk: SigningKey = generate_signing_key();
        let rogue_tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rogue_port = rogue_tcp.local_addr().unwrap().port();
        let rogue_udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let rogue_udp_addr = rogue_udp.local_addr().unwrap();

        let cancel_rogue = CancellationToken::new();
        let cancel_tcp = cancel_rogue.clone();
        let rogue_tcp_task = tokio::spawn(async move {
            serve_tcp_listener(rogue_tcp, cancel_tcp, 2, move |stream, _addr| {
                let sk = rogue_sk.clone();
                async move {
                    let _ = accept_exchange_on(stream, &sk, PROTOCOL_NAME, None, |_, _| true).await;
                }
            })
            .await;
        });

        let cancel_udp = cancel_rogue.clone();
        let b_pub_for_beacon = b_pub.clone();
        let rogue_udp_task = tokio::spawn(async move {
            let _ = announce_on(
                rogue_udp,
                cancel_udp,
                PROTOCOL_NAME.to_owned(),
                Arc::new(move || Beacon {
                    protocol: PROTOCOL_NAME.to_owned(),
                    device_id: b_pub_for_beacon.clone(),
                    name: "Fake B".to_owned(),
                    exchange_port: rogue_port,
                    pair_port: None,
                    meta: BTreeMap::new(),
                }),
            )
            .await;
        });

        let svc_a = LanSyncService::start(
            Arc::clone(&db_a),
            LanServiceConfig {
                device_name: "MacBook A".to_owned(),
                exchange_bind_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                discovery_bind_addr: Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
                discovery_targets: Some(vec![rogue_udp_addr]),
                discovery_timeout: Duration::from_millis(250),
                periodic_interval: Duration::from_secs(300),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let synced = svc_a.sync_now().await.unwrap();
        assert_eq!(synced, 0);
        let status_a = svc_a.status();
        assert_eq!(status_a.peers.len(), 1);
        assert!(status_a.peers[0].last_error.is_some());

        svc_a.shutdown().await;
        cancel_rogue.cancel();
        let _ = rogue_tcp_task.await;
        let _ = rogue_udp_task.await;
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[tokio::test]
    async fn manual_address_sync_works_without_udp_discovery_and_shuts_down_cleanly() {
        let (dir_a, path_a) = temp_db_path("man-a");
        let (dir_b, path_b) = temp_db_path("man-b");
        let db_a = Arc::new(Mutex::new(Database::open(&path_a).unwrap()));
        let db_b = Arc::new(Mutex::new(Database::open(&path_b).unwrap()));

        pair_two_databases(Arc::clone(&db_a), Arc::clone(&db_b), PeerRole::OwnDevice).await;
        let b_pub = db_b.lock().unwrap().sync_identity().device_public_key;

        let svc_b = LanSyncService::start(
            Arc::clone(&db_b),
            LanServiceConfig {
                device_name: "PC B".to_owned(),
                exchange_bind_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                discovery_bind_addr: None,
                periodic_interval: Duration::from_secs(300),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let b_addr = svc_b.exchange_addr();

        let svc_a = LanSyncService::start(
            Arc::clone(&db_a),
            LanServiceConfig {
                device_name: "MacBook A".to_owned(),
                exchange_bind_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                discovery_bind_addr: None,
                periodic_interval: Duration::from_secs(300),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        {
            let db = db_a.lock().unwrap();
            let list = db.create_shopping_list("Subotnja kupnja", None).unwrap();
            db.add_planned_item(
                list.id,
                None,
                "Maslinovo ulje",
                1.0,
                crate::models::Unit::L,
                890,
            )
            .unwrap();
        }

        let outcome = svc_a.sync_peer_at(&b_pub, b_addr).await.unwrap();
        assert!(outcome.converged);

        let lists_b = db_b.lock().unwrap().list_shopping_lists().unwrap();
        assert!(lists_b.iter().any(|l| l.name == "Subotnja kupnja"));

        svc_a.shutdown().await;
        svc_b.shutdown().await;

        // Exchange port on B must be freed after shutdown.
        let rebound = TcpListener::bind(b_addr).await;
        assert!(rebound.is_ok());

        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }
}
