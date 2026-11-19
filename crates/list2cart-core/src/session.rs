//! Transport-independent bidirectional sync conversation (`S09` — `peers/razgovor.go`).
//! Runs the exact same protocol over in-memory duplex streams, LAN TLS 1.3, QUIC, or tunnels.
use crate::{
    apply::PROTOCOL_NAME,
    db::Database,
    exchange::{SyncReceiveOutcome, SyncRequest},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use syncnet::{
    identity::public_key_to_base64,
    tls::{
        AuthenticatedStream, CredentialsProvider, TlsAuthError, accept_exchange_on,
        dial_exchange_on,
    },
    wire::{
        bundle_v2::ProtectedBundleV2,
        conversation::{
            ContentMessage, ConversationEnvelope, ConversationError, ConversationHello,
            DEFAULT_STEP_TIMEOUT, DoneMessage, KIND_CONTENT, KIND_DONE, KIND_HELLO, KIND_PAGE,
            KIND_REQUEST, KIND_WANTS, MAX_WANTS_FRAME_BYTES, RefusalCode, WantsMessage,
            exchange_step_by_role, receive_envelope_up_to, send_envelope,
        },
    },
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::timeout,
};

pub const DEFAULT_MAX_PAGE_BYTES: usize = 256 * 1024;
pub const DEFAULT_MAX_ROUNDS: usize = 128;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub protocol: String,
    pub max_page_bytes: usize,
    pub step_timeout: Duration,
    pub total_timeout: Duration,
    pub max_rounds: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            protocol: PROTOCOL_NAME.to_owned(),
            max_page_bytes: DEFAULT_MAX_PAGE_BYTES,
            step_timeout: DEFAULT_STEP_TIMEOUT,
            total_timeout: Duration::from_secs(300),
            max_rounds: DEFAULT_MAX_ROUNDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundOutcome {
    pub peer_hello: ConversationHello,
    pub applied_versions: usize,
    pub peer_applied_versions: usize,
    pub rejected_versions: usize,
    pub blocked_projections: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOutcome {
    pub rounds: usize,
    pub peer_hello: ConversationHello,
    pub total_applied_versions: usize,
    pub total_peer_applied_versions: usize,
    pub total_rejected_versions: usize,
    pub total_blocked_projections: usize,
    pub converged: bool,
}

pub fn default_conversation_hello(protocol: &str) -> ConversationHello {
    let mut vrste = BTreeMap::new();
    for kind in [
        "stores",
        "products",
        "price_history",
        "receipts",
        "receipt_items",
        "shopping_lists",
        "planned_items",
        "membership",
        "ovlast",
        "list_grant",
        "revocation",
    ] {
        vrste.insert(kind.to_owned(), 1);
    }
    ConversationHello {
        protocol: protocol.to_owned(),
        program: env!("CARGO_PKG_VERSION").to_owned(),
        gradnja: format!("{}-rust", env!("CARGO_PKG_VERSION")),
        mogucnosti: vec![
            "content/1".to_owned(),
            "family-sync/3".to_owned(),
            "own-sync/2".to_owned(),
            "rupe/1".to_owned(),
            "snapshot-pages/2".to_owned(),
        ],
        vrste,
    }
}

async fn send_refusal_best_effort<S>(
    stream: &mut S,
    code: RefusalCode,
    reason: impl Into<String>,
    step_timeout: Duration,
) where
    S: AsyncWrite + Unpin,
{
    let env = ConversationEnvelope::refusal(code, reason);
    let _ = send_envelope(stream, &env, step_timeout).await;
}

/// Executes one full conversation round (`Hello` -> `Request` -> `Page` -> `Apply` -> `Wants/Content` -> `Done`).
/// Strictly follows role ordering (`initiator` sends first, `!initiator` receives first) at every step.
/// Never holds the SQLite `Mutex<Database>` across `.await`.
pub async fn run_conversation_round<S>(
    stream: &mut S,
    db_mutex: &Mutex<Database>,
    peer_key: &[u8; 32],
    peer_credentials: Option<&[u8]>,
    initiator: bool,
    config: &SessionConfig,
) -> Result<RoundOutcome, ConversationError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let peer_b64 = public_key_to_base64(peer_key);

    // 1. Verify peer membership (recovering unilateral finish if needed) and read our node_id
    let step1_db: Result<String, (RefusalCode, String)> = {
        match db_mutex.try_lock() {
            Err(_) => Err((
                RefusalCode::WriteBusy,
                syncnet::wire::conversation::REASON_WRITE_BUSY_0038.to_owned(),
            )),
            Ok(db) => match db.verify_or_recover_peer_access(peer_key, peer_credentials) {
                Ok(true) => Ok(db.sync_identity().device_public_key),
                Ok(false) => Err((
                    RefusalCode::NotAMember,
                    "čvor nije aktivni član mreže ili je opozvan".to_owned(),
                )),
                Err(e) => Err((RefusalCode::Refused, e)),
            },
        }
    };
    let our_node_id = match step1_db {
        Ok(id) => id,
        Err((code, reason)) => {
            send_refusal_best_effort(stream, code, &reason, config.step_timeout).await;
            if matches!(code, RefusalCode::WriteBusy | RefusalCode::NotAMember) {
                return Err(ConversationError::RefusedByPeer { code, reason });
            }
            return Err(ConversationError::Application(reason));
        }
    };

    // 2. Step 1: Exchange Hello (protocol, version, capabilities, schema kinds)
    let our_hello = default_conversation_hello(&config.protocol);
    let raw_peer_hello: ConversationHello = exchange_step_by_role(
        stream,
        initiator,
        KIND_HELLO,
        &our_node_id,
        &our_hello,
        config.step_timeout,
    )
    .await?;
    let peer_hello = raw_peer_hello.sanitize_received();
    if peer_hello.protocol != config.protocol {
        let err = ConversationError::WrongProtocol {
            peer: peer_hello.protocol,
            expected: config.protocol.clone(),
        };
        send_refusal_best_effort(
            stream,
            RefusalCode::Refused,
            err.to_string(),
            config.step_timeout,
        )
        .await;
        return Err(err);
    }

    // 3. Step 2: Build and exchange signed SyncRequest (frontier, holes, snapshot progress, backfill)
    let req_db: Result<SyncRequest, (RefusalCode, String)> = {
        match db_mutex.try_lock() {
            Err(_) => Err((
                RefusalCode::WriteBusy,
                syncnet::wire::conversation::REASON_WRITE_BUSY_0038.to_owned(),
            )),
            Ok(db) => db
                .sync_request(&peer_b64)
                .map_err(|e| (RefusalCode::Refused, e)),
        }
    };
    let our_request = match req_db {
        Ok(req) => req,
        Err((code, reason)) => {
            send_refusal_best_effort(stream, code, &reason, config.step_timeout).await;
            if code == RefusalCode::WriteBusy {
                return Err(ConversationError::RefusedByPeer { code, reason });
            }
            return Err(ConversationError::Application(reason));
        }
    };

    let their_request: SyncRequest = exchange_step_by_role(
        stream,
        initiator,
        KIND_REQUEST,
        &our_node_id,
        &our_request,
        config.step_timeout,
    )
    .await?;

    if their_request.sender != peer_b64 || their_request.recipient != our_node_id {
        let msg = "Potpisani zahtjev za razmjenu ne odgovara identitetu TLS sesije";
        send_refusal_best_effort(stream, RefusalCode::Refused, msg, config.step_timeout).await;
        return Err(ConversationError::Application(msg.to_owned()));
    }

    // 4. Step 3: Build and exchange ProtectedBundleV2 (delta or snapshot page) in role order
    let reply_db: Result<ProtectedBundleV2, (RefusalCode, String)> = {
        match db_mutex.try_lock() {
            Err(_) => Err((
                RefusalCode::WriteBusy,
                syncnet::wire::conversation::REASON_WRITE_BUSY_0038.to_owned(),
            )),
            Ok(db) => db
                .sync_reply(&their_request, config.max_page_bytes)
                .map_err(|e| (RefusalCode::Refused, e)),
        }
    };
    let our_bundle = match reply_db {
        Ok(b) => b,
        Err((code, reason)) => {
            send_refusal_best_effort(stream, code, &reason, config.step_timeout).await;
            if code == RefusalCode::WriteBusy {
                return Err(ConversationError::RefusedByPeer { code, reason });
            }
            return Err(ConversationError::Application(reason));
        }
    };

    let their_bundle: ProtectedBundleV2 = exchange_step_by_role(
        stream,
        initiator,
        KIND_PAGE,
        &our_node_id,
        &our_bundle,
        config.step_timeout,
    )
    .await?;

    if their_bundle.sender_pub_key != peer_b64 || their_bundle.recipient_pub_key != our_node_id {
        let msg = "Zaštićena stranica ne odgovara identitetu TLS sesije";
        send_refusal_best_effort(stream, RefusalCode::Refused, msg, config.step_timeout).await;
        return Err(ConversationError::Application(msg.to_owned()));
    }

    // 5. Step 4: Validate, project, and commit received page under database lock
    let apply_db: Result<(usize, usize, usize, bool), (RefusalCode, String)> = {
        match db_mutex.try_lock() {
            Err(_) => Err((
                RefusalCode::WriteBusy,
                syncnet::wire::conversation::REASON_WRITE_BUSY_0038.to_owned(),
            )),
            Ok(db) => match db.import_sync_page(&their_bundle) {
                Ok(SyncReceiveOutcome::Delta { report, has_more }) => Ok((
                    report.applied_versions,
                    report.rejected.len(),
                    report.blocked_projections.len(),
                    has_more,
                )),
                Ok(SyncReceiveOutcome::Snapshot { progress }) => {
                    let applied = if progress.complete {
                        progress.total_versions
                    } else {
                        0
                    };
                    Ok((applied, 0, 0, true))
                }
                Err(e) => Err((RefusalCode::Refused, e)),
            },
        }
    };
    let (applied_versions, rejected_versions, blocked_projections, mut our_has_more) =
        match apply_db {
            Ok(vals) => vals,
            Err((code, reason)) => {
                send_refusal_best_effort(stream, code, &reason, config.step_timeout).await;
                if code == RefusalCode::WriteBusy {
                    return Err(ConversationError::RefusedByPeer { code, reason });
                }
                return Err(ConversationError::Application(reason));
            }
        };

    // 6. Step 5 & 6: Exchange Wants/Content (or early Done from legacy/no-content peer) and Done
    let our_wants = {
        let db = db_mutex
            .lock()
            .map_err(|_| ConversationError::Application("Greska zakljucavanja baze".into()))?;
        let otisci = db
            .pending_content_wants(crate::content::MAX_WANTS_PER_ROUND)
            .map_err(ConversationError::Application)?;
        WantsMessage { otisci }
    };
    let mut content_stored = 0usize;

    let their_done = if initiator {
        let wants_env = ConversationEnvelope::new(KIND_WANTS, &our_node_id, &our_wants)?;
        send_envelope(stream, &wants_env, config.step_timeout).await?;

        let next_env = receive_envelope_up_to(
            stream,
            KIND_WANTS,
            MAX_WANTS_FRAME_BYTES,
            config.step_timeout,
        )
        .await?;
        if next_env.kind == KIND_DONE {
            let d: DoneMessage = next_env.decode_payload()?;
            let our_done = DoneMessage {
                applied: applied_versions,
                has_more: our_has_more,
            };
            let done_env = ConversationEnvelope::new(KIND_DONE, &our_node_id, &our_done)?;
            send_envelope(stream, &done_env, config.step_timeout).await?;
            d
        } else if next_env.kind == KIND_WANTS {
            let their_wants: WantsMessage = next_env.decode_payload()?;
            let our_content = {
                let db = db_mutex.lock().map_err(|_| {
                    ConversationError::Application("Greska zakljucavanja baze".into())
                })?;
                let stavke = db
                    .fulfill_content_wants(peer_key, &their_wants.otisci)
                    .map_err(ConversationError::Application)?;
                ContentMessage { stavke }
            };
            let their_content: ContentMessage = exchange_step_by_role(
                stream,
                true,
                KIND_CONTENT,
                &our_node_id,
                &our_content,
                config.step_timeout,
            )
            .await?;
            let (stored_now, more_wants) = {
                let db = db_mutex.lock().map_err(|_| {
                    ConversationError::Application("Greska zakljucavanja baze".into())
                })?;
                let rep = db
                    .receive_wanted_content(&their_content.stavke)
                    .map_err(ConversationError::Application)?;
                let remaining = db
                    .pending_content_wants(1)
                    .map_err(ConversationError::Application)?;
                (rep.stored, !remaining.is_empty())
            };
            content_stored = stored_now;
            if content_stored > 0 && more_wants {
                our_has_more = true;
            }
            let our_done = DoneMessage {
                applied: applied_versions,
                has_more: our_has_more,
            };
            exchange_step_by_role(
                stream,
                true,
                KIND_DONE,
                &our_node_id,
                &our_done,
                config.step_timeout,
            )
            .await?
        } else {
            return Err(ConversationError::UnexpectedKind {
                expected: KIND_WANTS.to_owned(),
                actual: next_env.kind,
            });
        }
    } else {
        let next_env = receive_envelope_up_to(
            stream,
            KIND_WANTS,
            MAX_WANTS_FRAME_BYTES,
            config.step_timeout,
        )
        .await?;
        if next_env.kind == KIND_DONE {
            let d: DoneMessage = next_env.decode_payload()?;
            let our_done = DoneMessage {
                applied: applied_versions,
                has_more: our_has_more,
            };
            let done_env = ConversationEnvelope::new(KIND_DONE, &our_node_id, &our_done)?;
            send_envelope(stream, &done_env, config.step_timeout).await?;
            d
        } else if next_env.kind == KIND_WANTS {
            let their_wants: WantsMessage = next_env.decode_payload()?;
            let wants_env = ConversationEnvelope::new(KIND_WANTS, &our_node_id, &our_wants)?;
            send_envelope(stream, &wants_env, config.step_timeout).await?;

            let our_content = {
                let db = db_mutex.lock().map_err(|_| {
                    ConversationError::Application("Greska zakljucavanja baze".into())
                })?;
                let stavke = db
                    .fulfill_content_wants(peer_key, &their_wants.otisci)
                    .map_err(ConversationError::Application)?;
                ContentMessage { stavke }
            };
            let their_content: ContentMessage = exchange_step_by_role(
                stream,
                false,
                KIND_CONTENT,
                &our_node_id,
                &our_content,
                config.step_timeout,
            )
            .await?;
            let (stored_now, more_wants) = {
                let db = db_mutex.lock().map_err(|_| {
                    ConversationError::Application("Greska zakljucavanja baze".into())
                })?;
                let rep = db
                    .receive_wanted_content(&their_content.stavke)
                    .map_err(ConversationError::Application)?;
                let remaining = db
                    .pending_content_wants(1)
                    .map_err(ConversationError::Application)?;
                (rep.stored, !remaining.is_empty())
            };
            content_stored = stored_now;
            if content_stored > 0 && more_wants {
                our_has_more = true;
            }
            let our_done = DoneMessage {
                applied: applied_versions,
                has_more: our_has_more,
            };
            exchange_step_by_role(
                stream,
                false,
                KIND_DONE,
                &our_node_id,
                &our_done,
                config.step_timeout,
            )
            .await?
        } else {
            return Err(ConversationError::UnexpectedKind {
                expected: KIND_WANTS.to_owned(),
                actual: next_env.kind,
            });
        }
    };

    let _ = content_stored;
    Ok(RoundOutcome {
        peer_hello,
        applied_versions,
        peer_applied_versions: their_done.applied,
        rejected_versions,
        blocked_projections,
        has_more: our_has_more || their_done.has_more,
    })
}

/// Runs conversation rounds over `stream` until both sides report `has_more == false`
/// (or `config.max_rounds` is reached), bounded by `config.total_timeout`.
pub async fn run_conversation_until_idle<S>(
    stream: &mut S,
    db_mutex: &Mutex<Database>,
    peer_key: &[u8; 32],
    peer_credentials: Option<&[u8]>,
    initiator: bool,
    config: &SessionConfig,
) -> Result<SessionOutcome, ConversationError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    timeout(config.total_timeout, async {
        let mut rounds = 0usize;
        let mut total_applied = 0usize;
        let mut total_peer_applied = 0usize;
        let mut total_rejected = 0usize;
        let mut total_blocked = 0usize;
        let mut last_hello = ConversationHello::default();
        let max_rounds = config.max_rounds.max(1);

        while rounds < max_rounds {
            let round = run_conversation_round(
                stream,
                db_mutex,
                peer_key,
                if rounds == 0 { peer_credentials } else { None },
                initiator,
                config,
            )
            .await?;
            rounds += 1;
            total_applied += round.applied_versions;
            total_peer_applied += round.peer_applied_versions;
            total_rejected += round.rejected_versions;
            total_blocked += round.blocked_projections;
            last_hello = round.peer_hello;

            if !round.has_more {
                return Ok(SessionOutcome {
                    rounds,
                    peer_hello: last_hello,
                    total_applied_versions: total_applied,
                    total_peer_applied_versions: total_peer_applied,
                    total_rejected_versions: total_rejected,
                    total_blocked_projections: total_blocked,
                    converged: true,
                });
            }
        }

        Ok(SessionOutcome {
            rounds,
            peer_hello: last_hello,
            total_applied_versions: total_applied,
            total_peer_applied_versions: total_peer_applied,
            total_rejected_versions: total_rejected,
            total_blocked_projections: total_blocked,
            converged: false,
        })
    })
    .await
    .map_err(|_| ConversationError::Timeout {
        step: "ukupno trajanje razgovora".to_owned(),
    })?
}

/// Dials a TLS 1.3 exchange connection to `expected_peer_key` over `stream` and runs
/// the full conversation until both peers converge.
pub async fn dial_and_run_conversation_on<S>(
    stream: S,
    db_mutex: &Mutex<Database>,
    addr_label: &str,
    expected_peer_key: &[u8; 32],
    config: &SessionConfig,
) -> Result<SessionOutcome, ConversationError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (sk, creds) = {
        let db = db_mutex
            .lock()
            .map_err(|_| ConversationError::Application("Greska zakljucavanja baze".into()))?;
        (
            db.clone_device_signing_key(),
            db.tls_credentials()
                .map_err(ConversationError::Application)?,
        )
    };
    let creds_fn: Option<CredentialsProvider> = Some(Arc::new(move || creds.clone()));
    let mut auth: AuthenticatedStream<_> = dial_exchange_on(
        stream,
        &sk,
        &config.protocol,
        creds_fn,
        addr_label,
        expected_peer_key,
    )
    .await
    .map_err(|e| ConversationError::Io(e.to_string()))?;

    run_conversation_until_idle(
        &mut auth.stream,
        db_mutex,
        &auth.peer_key,
        auth.peer_credentials.as_deref(),
        true,
        config,
    )
    .await
}

/// Accepts a TLS 1.3 exchange connection on `stream` (verifying membership or recovering
/// unilateral pairing via X.509 SAN credentials) and runs the full conversation until idle.
pub async fn accept_and_run_conversation_on<S>(
    stream: S,
    db_mutex: Arc<Mutex<Database>>,
    config: &SessionConfig,
) -> Result<SessionOutcome, ConversationError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (sk, creds) = {
        let db = db_mutex
            .lock()
            .map_err(|_| ConversationError::Application("Greska zakljucavanja baze".into()))?;
        (
            db.clone_device_signing_key(),
            db.tls_credentials()
                .map_err(ConversationError::Application)?,
        )
    };
    let creds_fn: Option<CredentialsProvider> = Some(Arc::new(move || creds.clone()));
    let db_for_gate = db_mutex.clone();
    let mut auth: AuthenticatedStream<_> = accept_exchange_on(
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
    .map_err(|e| match e {
        TlsAuthError::UntrustedPeer { .. } => ConversationError::RefusedByPeer {
            code: RefusalCode::NotAMember,
            reason: e.to_string(),
        },
        other => ConversationError::Io(other.to_string()),
    })?;

    run_conversation_until_idle(
        &mut auth.stream,
        &db_mutex,
        &auth.peer_key,
        auth.peer_credentials.as_deref(),
        false,
        config,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ledger::PeerRole,
        models::{CartItem, Unit},
        pairing_session::WelcomePack,
    };
    use syncnet::{
        pairing::PairHello,
        wire::{
            conversation::{MAX_HELLO_FRAME_BYTES, expect_envelope},
            frame::write_frame_async,
        },
    };
    use tokio::{self, io::AsyncWriteExt};

    type PairedTestPair = (
        Arc<Mutex<Database>>,
        Arc<Mutex<Database>>,
        [u8; 32],
        [u8; 32],
    );

    fn make_paired_databases(role: PeerRole) -> PairedTestPair {
        let mut a = Database::open_in_memory().unwrap();
        let mut b = Database::open_in_memory().unwrap();
        let key_a = a.clone_device_signing_key().verifying_key().to_bytes();
        let key_b = b.clone_device_signing_key().verifying_key().to_bytes();

        let welcome_a = a.prepare_welcome_pack(&key_b, role, "*", true).unwrap();
        let welcome_b = b
            .prepare_welcome_pack(&key_a, PeerRole::OwnDevice, "*", false)
            .unwrap();
        let hello_a = PairHello {
            protocol: "list2cart".into(),
            device_id: a.sync_identity().device_public_key,
            name: "Uredjaj A".into(),
            version: "0.1.0".into(),
            meta: BTreeMap::new(),
            sas: 2,
        };
        let hello_b = PairHello {
            protocol: "list2cart".into(),
            device_id: b.sync_identity().device_public_key,
            name: "Uredjaj B".into(),
            version: "0.1.0".into(),
            meta: BTreeMap::new(),
            sas: 2,
        };

        a.finalize_pairing(
            &hello_b,
            &key_b,
            "111222",
            role,
            "*",
            Some(&welcome_a),
            Some(&welcome_b),
        )
        .unwrap();
        b.finalize_pairing(
            &hello_a,
            &key_a,
            "111222",
            PeerRole::OwnDevice,
            "*",
            Some(&WelcomePack::default()),
            Some(&welcome_a),
        )
        .unwrap();

        (
            Arc::new(Mutex::new(a)),
            Arc::new(Mutex::new(b)),
            key_a,
            key_b,
        )
    }

    #[tokio::test]
    async fn bidirectional_conversation_converges_over_multi_page_deltas_and_snapshots_in_both_roles()
     {
        for a_is_initiator in [true, false] {
            let (db_a, db_b, key_a, key_b) = make_paired_databases(PeerRole::OwnDevice);

            {
                let a = db_a.lock().unwrap();
                let list_a = a.create_shopping_list("Popis s A", None).unwrap();
                for i in 0..8 {
                    a.add_planned_item(
                        list_a.id,
                        None,
                        &format!("Artikl A-{i}"),
                        1.0,
                        Unit::Kom,
                        100 + i * 10,
                    )
                    .unwrap();
                }
            }
            {
                let b = db_b.lock().unwrap();
                let list_b = b.create_shopping_list("Popis s B", None).unwrap();
                for i in 0..8 {
                    b.add_planned_item(
                        list_b.id,
                        None,
                        &format!("Artikl B-{i}"),
                        2.0,
                        Unit::Kom,
                        200 + i * 10,
                    )
                    .unwrap();
                }
            }

            // Use a small 4 KiB duplex stream AND a small 3200 B page budget so both sides
            // must paginate across multiple rounds without deadlocking!
            let cfg = SessionConfig {
                max_page_bytes: 3200,
                ..SessionConfig::default()
            };
            let (mut io_a, mut io_b) = tokio::io::duplex(4096);
            let db_a_clone = db_a.clone();
            let db_b_clone = db_b.clone();
            let cfg_a = cfg.clone();
            let cfg_b = cfg.clone();

            let task_a = tokio::spawn(async move {
                run_conversation_until_idle(
                    &mut io_a,
                    &db_a_clone,
                    &key_b,
                    None,
                    a_is_initiator,
                    &cfg_a,
                )
                .await
            });
            let task_b = tokio::spawn(async move {
                run_conversation_until_idle(
                    &mut io_b,
                    &db_b_clone,
                    &key_a,
                    None,
                    !a_is_initiator,
                    &cfg_b,
                )
                .await
            });

            let (out_a, out_b) = tokio::join!(task_a, task_b);
            let out_a = out_a.unwrap().expect("conversation A");
            let out_b = out_b.unwrap().expect("conversation B");

            assert!(out_a.converged && out_b.converged);
            assert!(out_a.rounds >= 2, "must paginate over multiple rounds");
            assert_eq!(out_a.rounds, out_b.rounds);
            assert!(out_a.total_applied_versions >= 9);
            assert!(out_b.total_applied_versions >= 9);

            let lists_a = db_a.lock().unwrap().list_shopping_lists().unwrap();
            let lists_b = db_b.lock().unwrap().list_shopping_lists().unwrap();
            assert_eq!(lists_a.len(), 2);
            assert_eq!(lists_b.len(), 2);

            // Second conversation when already converged completes in 1 idle round with 0 applied
            let (mut io_a2, mut io_b2) = tokio::io::duplex(8192);
            let db_a2 = db_a.clone();
            let db_b2 = db_b.clone();
            let cfg2 = SessionConfig::default();
            let cfg2_b = cfg2.clone();
            let (idle_a, idle_b) = tokio::join!(
                tokio::spawn(async move {
                    run_conversation_until_idle(
                        &mut io_a2,
                        &db_a2,
                        &key_b,
                        None,
                        a_is_initiator,
                        &cfg2,
                    )
                    .await
                }),
                tokio::spawn(async move {
                    run_conversation_until_idle(
                        &mut io_b2,
                        &db_b2,
                        &key_a,
                        None,
                        !a_is_initiator,
                        &cfg2_b,
                    )
                    .await
                }),
            );
            let idle_a = idle_a.unwrap().unwrap();
            let idle_b = idle_b.unwrap().unwrap();
            assert_eq!(idle_a.rounds, 1);
            assert_eq!(idle_b.rounds, 1);
            assert_eq!(idle_a.total_applied_versions, 0);
            assert_eq!(idle_b.total_applied_versions, 0);
        }
    }

    #[tokio::test]
    async fn contract_tests_for_both_roles_reject_wrong_order_oversized_truncated_and_refusal_without_false_ack()
     {
        for initiator in [true, false] {
            let (db_a, db_b, _key_a, key_b) = make_paired_databases(PeerRole::OwnDevice);
            {
                let a = db_a.lock().unwrap();
                a.create_shopping_list("Nepotvrdeni popis", None).unwrap();
            }
            let cfg = SessionConfig {
                step_timeout: Duration::from_secs(2),
                ..SessionConfig::default()
            };

            // 1. Wrong message order (peer sends `done` when `hello` is expected)
            {
                let (mut node_io, mut script_io) = tokio::io::duplex(16 * 1024);
                let db_node = db_a.clone();
                let cfg_node = cfg.clone();
                let node_task = tokio::spawn(async move {
                    run_conversation_round(
                        &mut node_io,
                        &db_node,
                        &key_b,
                        None,
                        initiator,
                        &cfg_node,
                    )
                    .await
                });
                let script_task = tokio::spawn(async move {
                    if initiator {
                        let _: ConversationHello =
                            expect_envelope(&mut script_io, KIND_HELLO, Duration::from_secs(2))
                                .await
                                .unwrap();
                    }
                    let wrong = ConversationEnvelope::new(
                        KIND_DONE,
                        "peer",
                        &DoneMessage {
                            applied: 0,
                            has_more: false,
                        },
                    )
                    .unwrap();
                    send_envelope(&mut script_io, &wrong, Duration::from_secs(2))
                        .await
                        .unwrap();
                });
                let _ = script_task.await;
                let res = node_task.await.unwrap();
                assert!(matches!(
                    res,
                    Err(ConversationError::UnexpectedKind {
                        ref expected,
                        ref actual
                    }) if expected == KIND_HELLO && actual == KIND_DONE
                ));
            }

            // 2. Oversized frame header (claims 100 KiB on `hello` whose max is 64 KiB -> rejected before allocation)
            {
                let (mut node_io, mut script_io) = tokio::io::duplex(16 * 1024);
                let db_node = db_a.clone();
                let cfg_node = cfg.clone();
                let node_task = tokio::spawn(async move {
                    run_conversation_round(
                        &mut node_io,
                        &db_node,
                        &key_b,
                        None,
                        initiator,
                        &cfg_node,
                    )
                    .await
                });
                let script_task = tokio::spawn(async move {
                    if initiator {
                        let _: ConversationHello =
                            expect_envelope(&mut script_io, KIND_HELLO, Duration::from_secs(2))
                                .await
                                .unwrap();
                    }
                    let oversized_len = ((MAX_HELLO_FRAME_BYTES as u32) + 1024).to_be_bytes();
                    let _ = script_io.write_all(&oversized_len).await;
                    let _ = script_io.shutdown().await;
                });
                let _ = script_task.await;
                let res = node_task.await.unwrap();
                assert!(matches!(
                    res,
                    Err(ConversationError::MessageTooLarge { ref kind, .. }) if kind == KIND_HELLO
                ));
            }

            // 3. Peer sends Refusal envelope (`KodZauzet`) instead of `request`
            {
                let (mut node_io, mut script_io) = tokio::io::duplex(16 * 1024);
                let db_node = db_a.clone();
                let cfg_node = cfg.clone();
                let node_task = tokio::spawn(async move {
                    run_conversation_round(
                        &mut node_io,
                        &db_node,
                        &key_b,
                        None,
                        initiator,
                        &cfg_node,
                    )
                    .await
                });
                let script_task = tokio::spawn(async move {
                    let hello = default_conversation_hello("list2cart");
                    let _: ConversationHello = exchange_step_by_role(
                        &mut script_io,
                        !initiator,
                        KIND_HELLO,
                        "peer",
                        &hello,
                        Duration::from_secs(2),
                    )
                    .await
                    .unwrap();
                    if initiator {
                        let _: SyncRequest =
                            expect_envelope(&mut script_io, KIND_REQUEST, Duration::from_secs(2))
                                .await
                                .unwrap();
                    }
                    let refusal = ConversationEnvelope::refusal(
                        RefusalCode::Busy,
                        syncnet::wire::conversation::REASON_BUSY_0038,
                    );
                    send_envelope(&mut script_io, &refusal, Duration::from_secs(2))
                        .await
                        .unwrap();
                });
                let _ = script_task.await;
                let res = node_task.await.unwrap();
                assert!(matches!(
                    res,
                    Err(ConversationError::RefusedByPeer {
                        code: RefusalCode::Busy,
                        ..
                    })
                ));
            }

            // 4. Truncated stream after `page` (before `done`) -> no false ACK on B, subsequent full sync still delivers A's list!
            {
                let (mut node_io, mut script_io) = tokio::io::duplex(16 * 1024);
                let db_node = db_a.clone();
                let cfg_node = cfg.clone();
                let req_b = db_b
                    .lock()
                    .unwrap()
                    .sync_request(&db_a.lock().unwrap().sync_identity().device_public_key)
                    .unwrap();
                let node_task = tokio::spawn(async move {
                    run_conversation_round(
                        &mut node_io,
                        &db_node,
                        &key_b,
                        None,
                        initiator,
                        &cfg_node,
                    )
                    .await
                });
                let script_task = tokio::spawn(async move {
                    let hello = default_conversation_hello("list2cart");
                    let _: ConversationHello = exchange_step_by_role(
                        &mut script_io,
                        !initiator,
                        KIND_HELLO,
                        "peer",
                        &hello,
                        Duration::from_secs(2),
                    )
                    .await
                    .unwrap();
                    let _req_a: SyncRequest = exchange_step_by_role(
                        &mut script_io,
                        !initiator,
                        KIND_REQUEST,
                        "peer",
                        &req_b,
                        Duration::from_secs(2),
                    )
                    .await
                    .unwrap();
                    // Truncate connection midway through `page` step without applying or sending `done`
                    let _ = write_frame_async(&mut script_io, b"{\"kind\":\"page\"", 1024).await;
                    let _ = script_io.shutdown().await;
                });
                let _ = script_task.await;
                assert!(node_task.await.unwrap().is_err());

                // Verify B still has 0 lists and A has NOT falsely marked B as having ACKed A's list:
                // running a normal conversation now must deliver "Nepotvrdeni popis" to B!
                let (mut ok_a_io, mut ok_b_io) = tokio::io::duplex(16 * 1024);
                let db_a_ok = db_a.clone();
                let db_b_ok = db_b.clone();
                let key_a = db_a
                    .lock()
                    .unwrap()
                    .clone_device_signing_key()
                    .verifying_key()
                    .to_bytes();
                let cfg_ok = cfg.clone();
                let cfg_ok2 = cfg.clone();
                let (ra, rb) = tokio::join!(
                    tokio::spawn(async move {
                        run_conversation_until_idle(
                            &mut ok_a_io,
                            &db_a_ok,
                            &key_b,
                            None,
                            initiator,
                            &cfg_ok,
                        )
                        .await
                    }),
                    tokio::spawn(async move {
                        run_conversation_until_idle(
                            &mut ok_b_io,
                            &db_b_ok,
                            &key_a,
                            None,
                            !initiator,
                            &cfg_ok2,
                        )
                        .await
                    }),
                );
                assert!(ra.unwrap().unwrap().converged);
                assert!(rb.unwrap().unwrap().converged);
                let lists_b = db_b.lock().unwrap().list_shopping_lists().unwrap();
                assert_eq!(lists_b.len(), 1);
                assert_eq!(lists_b[0].name, "Nepotvrdeni popis");
            }
        }
    }

    #[tokio::test]
    async fn tls13_authenticated_conversation_recovers_unilateral_pairing_and_syncs_end_to_end() {
        let admin = Database::open_in_memory().unwrap();
        let mut joiner = Database::open_in_memory().unwrap();

        let admin_key = admin.clone_device_signing_key().verifying_key().to_bytes();
        let joiner_key = joiner.clone_device_signing_key().verifying_key().to_bytes();

        // Simulate unilateral pairing: Joiner committed `welcome`, Admin did NOT commit `finalize_pairing`
        let welcome = admin
            .prepare_welcome_pack(&joiner_key, PeerRole::OwnDevice, "*", true)
            .unwrap();
        let admin_hello = PairHello {
            protocol: "list2cart".into(),
            device_id: admin.sync_identity().device_public_key,
            name: "Admin".into(),
            version: "0.1.0".into(),
            meta: BTreeMap::new(),
            sas: 2,
        };
        joiner
            .finalize_pairing(
                &admin_hello,
                &admin_key,
                "999888",
                PeerRole::OwnDevice,
                "*",
                None,
                Some(&welcome),
            )
            .unwrap();

        admin.create_shopping_list("Popis s Admina", None).unwrap();
        joiner
            .create_shopping_list("Popis s Joinera", None)
            .unwrap();

        let admin_mutex = Arc::new(Mutex::new(admin));
        let joiner_mutex = Arc::new(Mutex::new(joiner));
        let cfg = SessionConfig::default();
        let cfg_srv = cfg.clone();

        let (c_io, s_io) = tokio::io::duplex(32 * 1024);
        let admin_for_srv = admin_mutex.clone();
        let srv_task = tokio::spawn(async move {
            accept_and_run_conversation_on(s_io, admin_for_srv, &cfg_srv).await
        });
        let cli_out =
            dial_and_run_conversation_on(c_io, &joiner_mutex, "127.0.0.1:4610", &admin_key, &cfg)
                .await
                .expect("client TLS conversation");
        let srv_out = srv_task.await.unwrap().expect("server TLS conversation");

        assert!(cli_out.converged && srv_out.converged);
        assert_eq!(
            admin_mutex
                .lock()
                .unwrap()
                .list_shopping_lists()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            joiner_mutex
                .lock()
                .unwrap()
                .list_shopping_lists()
                .unwrap()
                .len(),
            2
        );
    }

    /// Pairs `joiner` into `admin`'s network with the given role, as the app does.
    fn pair_into(admin: &mut Database, joiner: &mut Database, role: PeerRole, name: &str) {
        let admin_key = admin.clone_device_signing_key().verifying_key().to_bytes();
        let joiner_key = joiner.clone_device_signing_key().verifying_key().to_bytes();
        let hello = |db: &Database, name: &str| PairHello {
            protocol: "list2cart".into(),
            device_id: db.sync_identity().device_public_key,
            name: name.into(),
            version: "0.1.0".into(),
            meta: BTreeMap::new(),
            sas: 2,
        };
        let (admin_hello, joiner_hello) = (hello(admin, "Vlasnik"), hello(joiner, name));
        let welcome = admin
            .prepare_welcome_pack(&joiner_key, role, "*", true)
            .unwrap();
        let reply = joiner
            .prepare_welcome_pack(&admin_key, PeerRole::OwnDevice, "*", false)
            .unwrap();
        admin
            .finalize_pairing(
                &joiner_hello,
                &joiner_key,
                "111222",
                role,
                "*",
                Some(&welcome),
                Some(&reply),
            )
            .unwrap();
        joiner
            .finalize_pairing(
                &admin_hello,
                &admin_key,
                "111222",
                PeerRole::OwnDevice,
                "*",
                Some(&WelcomePack::default()),
                Some(&welcome),
            )
            .unwrap();
    }

    async fn sync_pair(a: &Arc<Mutex<Database>>, b: &Arc<Mutex<Database>>) -> Result<(), String> {
        let key = |db: &Arc<Mutex<Database>>| {
            db.lock()
                .unwrap()
                .clone_device_signing_key()
                .verifying_key()
                .to_bytes()
        };
        let (key_a, key_b) = (key(a), key(b));
        let (mut io_a, mut io_b) = tokio::io::duplex(64 * 1024);
        let (a2, b2) = (a.clone(), b.clone());
        let cfg = SessionConfig::default();
        let (cfg_a, cfg_b) = (cfg.clone(), cfg);
        let (out_a, out_b) = tokio::join!(
            tokio::spawn(async move {
                run_conversation_until_idle(&mut io_a, &a2, &key_b, None, true, &cfg_a).await
            }),
            tokio::spawn(async move {
                run_conversation_until_idle(&mut io_b, &b2, &key_a, None, false, &cfg_b).await
            }),
        );
        match (out_a.unwrap(), out_b.unwrap()) {
            (Ok(_), Ok(_)) => Ok(()),
            (a, b) => Err(format!("A: {:?}; B: {:?}", a.err(), b.err())),
        }
    }

    struct ThreeDevices {
        owner: Arc<Mutex<Database>>,
        own: Arc<Mutex<Database>>,
        family: Arc<Mutex<Database>>,
        shared_list: String,
    }

    /// One network: its owner, the owner's second device and a family member's device.
    /// The owner's shared list "Zajednicki" exists before pairing, so it is opened to
    /// both devices as they join.
    fn three_devices() -> ThreeDevices {
        let mut owner = Database::open_in_memory().unwrap();
        let store = owner.add_store("Konzum", "Zagreb", "#c00").unwrap();
        let shared = owner
            .create_shared_shopping_list("Zajednicki", Some(store.id))
            .unwrap();
        assert!(shared.shared);
        let mut own = Database::open_in_memory().unwrap();
        let mut family = Database::open_in_memory().unwrap();
        pair_into(
            &mut owner,
            &mut own,
            PeerRole::OwnDevice,
            "Moj drugi uredjaj",
        );
        pair_into(
            &mut owner,
            &mut family,
            PeerRole::FamilyMember,
            "Clan obitelji",
        );
        ThreeDevices {
            owner: Arc::new(Mutex::new(owner)),
            own: Arc::new(Mutex::new(own)),
            family: Arc::new(Mutex::new(family)),
            shared_list: shared.sync_id,
        }
    }

    /// Creates a list with an item, a store and a receipt, as everyday use does.
    fn use_the_app(db: &Database, who: &str) -> Result<(), String> {
        let list = db.create_shopping_list(&format!("Popis od {who}"), None)?;
        db.add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 199)?;
        let store = db.add_store(&format!("Trgovina {who}"), "Split", "#0a0")?;
        let item = CartItem {
            id: "1".into(),
            barcode: "3850000000001".into(),
            name: "Jaja".into(),
            quantity: 1.0,
            unit: Unit::Kom,
            unit_price_cents: 250,
            total_cents: 250,
            image_data_url: None,
        };
        db.save_receipt(store.id, &store.name, &[item], 250)?;
        Ok(())
    }

    fn list_names(db: &Arc<Mutex<Database>>) -> Vec<String> {
        let mut names: Vec<_> = db
            .lock()
            .unwrap()
            .list_shopping_lists()
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
        names.sort();
        names
    }

    fn items_of(db: &Arc<Mutex<Database>>, list_sync_id: &str) -> Vec<String> {
        let list = db
            .lock()
            .unwrap()
            .list_shopping_lists()
            .unwrap()
            .into_iter()
            .find(|l| l.sync_id == list_sync_id)
            .unwrap_or_else(|| panic!("list {list_sync_id} is missing"));
        let mut names: Vec<_> = list.items.into_iter().map(|i| i.name).collect();
        names.sort();
        names
    }

    fn add_item(db: &Arc<Mutex<Database>>, list_sync_id: &str, name: &str) -> Result<(), String> {
        let db = db.lock().unwrap();
        let list = db
            .list_shopping_lists()?
            .into_iter()
            .find(|l| l.sync_id == list_sync_id)
            .ok_or("popis nije stigao na uredjaj")?;
        db.add_planned_item(list.id, None, name, 1.0, Unit::Kom, 99)
            .map(|_| ())
    }

    #[tokio::test]
    async fn owner_and_own_device_use_the_app_and_see_each_others_lists_and_receipts() {
        let d = three_devices();
        sync_pair(&d.owner, &d.own).await.unwrap();
        use_the_app(&d.owner.lock().unwrap(), "vlasnik").unwrap();
        use_the_app(&d.own.lock().unwrap(), "moj uredjaj").unwrap();
        add_item(&d.owner, &d.shared_list, "Mlijeko vlasnik").unwrap();
        add_item(&d.own, &d.shared_list, "Mlijeko moj uredjaj").unwrap();
        sync_pair(&d.owner, &d.own).await.unwrap();
        for db in [&d.owner, &d.own] {
            assert_eq!(
                list_names(db),
                ["Popis od moj uredjaj", "Popis od vlasnik", "Zajednicki"]
            );
            assert_eq!(db.lock().unwrap().list_receipts().unwrap().len(), 2);
            assert_eq!(
                items_of(db, &d.shared_list),
                ["Mlijeko moj uredjaj", "Mlijeko vlasnik"]
            );
        }
    }

    #[tokio::test]
    async fn family_member_uses_the_app_privately_and_shares_only_the_shared_list() {
        let d = three_devices();
        sync_pair(&d.owner, &d.family).await.unwrap();
        sync_pair(&d.owner, &d.own).await.unwrap();

        // Their own lists, stores and receipts work, and stay on their device.
        use_the_app(&d.family.lock().unwrap(), "clan obitelji").unwrap();
        assert_eq!(
            list_names(&d.family),
            ["Popis od clan obitelji", "Zajednicki"]
        );
        // The shared list arrived; what they add reaches the owner and the owner's devices.
        add_item(&d.family, &d.shared_list, "Jogurt clan").unwrap();
        add_item(&d.owner, &d.shared_list, "Mlijeko vlasnik").unwrap();
        sync_pair(&d.owner, &d.family).await.unwrap();
        sync_pair(&d.owner, &d.own).await.unwrap();
        for db in [&d.owner, &d.own, &d.family] {
            assert_eq!(
                items_of(db, &d.shared_list),
                ["Jogurt clan", "Mlijeko vlasnik"]
            );
        }

        // Nothing private crosses between the owner and the family member.
        use_the_app(&d.owner.lock().unwrap(), "vlasnik").unwrap();
        sync_pair(&d.owner, &d.family).await.unwrap();
        assert_eq!(list_names(&d.owner), ["Popis od vlasnik", "Zajednicki"]);
        assert_eq!(d.owner.lock().unwrap().list_receipts().unwrap().len(), 1);
        assert_eq!(
            list_names(&d.family),
            ["Popis od clan obitelji", "Zajednicki"]
        );
        assert_eq!(d.family.lock().unwrap().list_receipts().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_list_shared_after_pairing_reaches_every_member_and_only_the_owner_can_share() {
        let d = three_devices();
        sync_pair(&d.owner, &d.own).await.unwrap();
        sync_pair(&d.owner, &d.family).await.unwrap();
        let later = d
            .owner
            .lock()
            .unwrap()
            .create_shared_shopping_list("Vikend", None)
            .unwrap()
            .sync_id;
        sync_pair(&d.owner, &d.family).await.unwrap();
        sync_pair(&d.owner, &d.own).await.unwrap();
        add_item(&d.family, &later, "Ugljen").unwrap();
        add_item(&d.own, &later, "Kobasice").unwrap();
        sync_pair(&d.owner, &d.family).await.unwrap();
        sync_pair(&d.owner, &d.own).await.unwrap();
        sync_pair(&d.owner, &d.family).await.unwrap();
        for db in [&d.owner, &d.own, &d.family] {
            assert_eq!(items_of(db, &later), ["Kobasice", "Ugljen"]);
            assert!(
                db.lock()
                    .unwrap()
                    .list_shopping_lists()
                    .unwrap()
                    .iter()
                    .find(|l| l.sync_id == later)
                    .unwrap()
                    .shared
            );
        }
        // Granting needs the network key or a delegation, which neither device holds.
        for db in [&d.family, &d.own] {
            let refused = db.lock().unwrap().create_shared_shopping_list("Moj", None);
            assert!(
                refused
                    .unwrap_err()
                    .contains("samo uredjaj koji upravlja mrezom")
            );
        }
    }
}
