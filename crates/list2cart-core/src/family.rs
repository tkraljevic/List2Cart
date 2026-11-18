//! Recipient-scoped proof replay. A filtered control view has no global frontier:
//! its signed proofs accompany every page and never acknowledge unseen controls.
use crate::ledger::{
    DeltaBatch, HoleRecord, Version, compute_delta_paged_filtered, read_version_row,
    serialized_json_len,
};
use crate::models::ControlPayload;
use ed25519_dalek::VerifyingKey;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default, Clone)]
struct Needed {
    members: BTreeSet<String>,
    admins: BTreeSet<String>,
    grants: BTreeSet<String>,
}

impl Needed {
    fn includes(
        &self,
        ctrl: &ControlPayload,
        recipient: &str,
        channels: &BTreeSet<String>,
    ) -> bool {
        match ctrl {
            ControlPayload::Membership(m) => self.members.contains(&m.device_key),
            ControlPayload::Ovlast(o) => self.admins.contains(&o.device_key),
            ControlPayload::ListGrant(g) => {
                g.recipient_pub_key == recipient
                    || (self.grants.contains(&g.grant_id)
                        && channels.contains(&format!("list:{}", g.list_sync_id)))
            }
            ControlPayload::Revocation(r) => match r.scope() {
                Ok("membership") => {
                    self.members.contains(&r.public_key) || self.admins.contains(&r.public_key)
                }
                Ok("ovlast") => self.admins.contains(&r.public_key),
                Ok("list_grant") => self.grants.contains(&r.target_id),
                _ => {
                    self.members.contains(&r.public_key)
                        || self.admins.contains(&r.public_key)
                        || self.grants.contains(&r.target_id)
                }
            },
        }
    }

    fn extend(&mut self, ctrl: &ControlPayload, root: &str) {
        let admin = match ctrl {
            ControlPayload::Membership(m) => m.admitter.as_ref(),
            ControlPayload::ListGrant(g) => {
                self.grants.insert(g.grant_id.clone());
                g.admitter.as_ref()
            }
            ControlPayload::Revocation(r) => {
                if !r.signer_key.is_empty() && r.signer_key != root {
                    self.admins.insert(r.signer_key.clone());
                }
                None
            }
            _ => None,
        };
        if let Some(admin) = admin {
            self.admins.insert(admin.device_key.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn page(
    conn: &Connection,
    network_pub: &VerifyingKey,
    recipient: &str,
    root: &str,
    frontier: &BTreeMap<String, String>,
    holes: &[HoleRecord],
    channels: &BTreeSet<String>,
    budget: usize,
    now_unix: i64,
) -> Result<DeltaBatch, String> {
    if !channels.contains("control") {
        return compute_delta_paged_filtered(conn, frontier, holes, &BTreeSet::new(), budget, None);
    }
    let mut proofs = proofs(conn, network_pub, recipient, root, channels, now_unix)?;
    // Reserve the complete replayed proof set BEFORE domain pagination. One
    // extra comma joins the two version arrays (conservative if data is empty).
    let reserve = serialized_json_len(&proofs)? - 2 + usize::from(!proofs.is_empty());
    let available = budget
        .checked_sub(reserve)
        .filter(|n| *n > 0)
        .ok_or("Kontrolni dokazi ne stanu u obiteljsku stranicu; povećajte granicu do 1 MiB")?;
    let domain_channels: BTreeSet<String> = channels
        .iter()
        .filter(|c| c.as_str() != "control")
        .cloned()
        .collect();
    let relayable_domain_ids: BTreeSet<String> =
        crate::apply::relayable_domain_versions(conn, network_pub, &domain_channels, now_unix)?
            .into_iter()
            .map(|v| v.version_id)
            .collect();
    let mut batch = compute_delta_paged_filtered(
        conn,
        frontier,
        holes,
        &domain_channels,
        available,
        Some(&relayable_domain_ids),
    )?;
    proofs.append(&mut batch.versions);
    batch.versions = proofs;
    if serialized_json_len(&batch)? > budget {
        return Err("Obiteljska stranica prelazi granicu".into());
    }
    Ok(batch)
}

/// Signed dependency closure shared by replayed delta proofs and durable snapshots.
pub(crate) fn proofs(
    conn: &Connection,
    network_pub: &VerifyingKey,
    recipient: &str,
    root: &str,
    channels: &BTreeSet<String>,
    now_unix: i64,
) -> Result<Vec<Version>, String> {
    let mut required_needed = Needed::default();
    if recipient != root {
        required_needed.members.insert(recipient.to_owned());
    }
    let relayable_domains =
        crate::apply::relayable_domain_versions(conn, network_pub, channels, now_unix)?;
    for v in &relayable_domains {
        if v.author_pub_key != root {
            required_needed.members.insert(v.author_pub_key.clone());
            if v.channel.starts_with("list:") {
                required_needed.grants.insert(v.grant_ref.clone());
            }
        }
    }

    let mut all_needed = required_needed.clone();
    let mut statement = conn.prepare(
        "SELECT author_pub_key,grant_ref,channel FROM record_versions WHERE channel!='control' AND signature!='' AND legacy_imported=0"
    ).map_err(|e| e.to_string())?;
    let domains = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for domain in domains {
        let (author, grant, channel) = domain.map_err(|e| e.to_string())?;
        if channels.contains(&channel) && author != root {
            all_needed.members.insert(author);
            if channel.starts_with("list:") {
                all_needed.grants.insert(grant);
            }
        }
    }

    let mut statement = conn.prepare(
        "SELECT version_id,entity,entity_id,node_id,supersedes,archived,payload,created_at,schema_version,channel,author_pub_key,grant_ref,signature,created_at_unix
         FROM record_versions WHERE channel='control' AND signature!='' AND legacy_imported=0 ORDER BY version_id"
    ).map_err(|e|e.to_string())?;
    let controls = statement
        .query_map([], read_version_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<Version>, _>>()
        .map_err(|e| e.to_string())?;
    let relayable_ctrl_ids: BTreeSet<String> =
        crate::apply::relayable_control_versions(conn, network_pub, now_unix)?
            .into_iter()
            .map(|v| v.version_id)
            .collect();
    let mut entries = Vec::new();
    for version in controls {
        let ctrl: ControlPayload =
            serde_json::from_str(&version.payload).map_err(|e| e.to_string())?;
        let is_relayable = relayable_ctrl_ids.contains(&version.version_id);
        entries.push((version, ctrl, is_relayable));
    }
    // Include cache-only proofs in dependency discovery, but never manufacture
    // an original author's signed envelope for them.
    let mut statement = conn
        .prepare("SELECT raw_json FROM sync_trust_records")
        .map_err(|e| e.to_string())?;
    let cached = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let cached = cached
        .iter()
        .map(|raw| {
            let ctrl = serde_json::from_str::<ControlPayload>(raw).map_err(|e| e.to_string())?;
            let is_relayable =
                crate::apply::control_payload_is_relayable(conn, network_pub, &ctrl, now_unix)
                    .is_ok();
            Ok((ctrl, is_relayable))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut selected = BTreeSet::new();
    loop {
        let before = (
            required_needed.members.len(),
            required_needed.admins.len(),
            required_needed.grants.len(),
            all_needed.members.len(),
            all_needed.admins.len(),
            all_needed.grants.len(),
            selected.len(),
        );
        for (index, (version, ctrl, is_relayable)) in entries.iter().enumerate() {
            if *is_relayable && required_needed.includes(ctrl, recipient, channels) {
                required_needed.extend(ctrl, root);
                if version.author_pub_key != root {
                    required_needed
                        .admins
                        .insert(version.author_pub_key.clone());
                }
            }
            if all_needed.includes(ctrl, recipient, channels) {
                if *is_relayable {
                    selected.insert(index);
                }
                all_needed.extend(ctrl, root);
                if version.author_pub_key != root {
                    all_needed.admins.insert(version.author_pub_key.clone());
                }
            }
        }
        for (ctrl, is_relayable) in &cached {
            if *is_relayable && required_needed.includes(ctrl, recipient, channels) {
                required_needed.extend(ctrl, root);
            }
            if all_needed.includes(ctrl, recipient, channels) {
                all_needed.extend(ctrl, root);
            }
        }
        let after = (
            required_needed.members.len(),
            required_needed.admins.len(),
            required_needed.grants.len(),
            all_needed.members.len(),
            all_needed.admins.len(),
            all_needed.grants.len(),
            selected.len(),
        );
        if before == after {
            break;
        }
    }
    let mut proofs = Vec::new();
    let mut raw_proofs = BTreeSet::new();
    let mut members = BTreeSet::new();
    let mut admins = BTreeSet::new();
    let mut grants = BTreeSet::new();
    for index in selected {
        let (version, ctrl, _) = &entries[index];
        if let ControlPayload::Revocation(r) = ctrl
            && r.scope().is_err()
        {
            return Err("Legacy opoziv nema siguran opseg za obiteljski paket".into());
        }
        raw_proofs.insert(serde_json::to_string(ctrl).map_err(|e| e.to_string())?);
        match ctrl {
            ControlPayload::Membership(m) => {
                members.insert(m.device_key.clone());
            }
            ControlPayload::Ovlast(o) => {
                admins.insert(o.device_key.clone());
            }
            ControlPayload::ListGrant(g) => {
                grants.insert(g.grant_id.clone());
            }
            _ => {}
        }
        proofs.push(version.clone());
    }
    if !required_needed.members.is_subset(&members)
        || !required_needed.admins.is_subset(&admins)
        || !required_needed.grants.is_subset(&grants)
    {
        return Err("Nedostaje potpisani kontrolni dokaz za obiteljski paket".into());
    }
    for (ctrl, is_relayable) in cached {
        if is_relayable
            && required_needed.includes(&ctrl, recipient, channels)
            && !raw_proofs.contains(&serde_json::to_string(&ctrl).map_err(|e| e.to_string())?)
        {
            return Err(
                "Kontrolni dokaz postoji samo u cacheu; nedostaje izvorna potpisana verzija".into(),
            );
        }
    }
    Ok(proofs)
}
