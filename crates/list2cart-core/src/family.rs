//! Recipient-scoped proof replay. A filtered control view has no global frontier:
//! its signed proofs accompany every page and never acknowledge unseen controls.
use crate::ledger::{
    DeltaBatch, HoleRecord, Version, compute_delta_paged, read_version_row, serialized_json_len,
};
use crate::models::ControlPayload;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
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

    fn extend(&mut self, ctrl: &ControlPayload) {
        let admin = match ctrl {
            ControlPayload::Membership(m) => m.admitter.as_ref(),
            ControlPayload::ListGrant(g) => {
                self.grants.insert(g.grant_id.clone());
                g.admitter.as_ref()
            }
            _ => None,
        };
        if let Some(admin) = admin {
            self.admins.insert(admin.device_key.clone());
        }
    }
}

pub(crate) fn page(
    conn: &Connection,
    recipient: &str,
    root: &str,
    frontier: &BTreeMap<String, String>,
    holes: &[HoleRecord],
    channels: &BTreeSet<String>,
    budget: usize,
) -> Result<DeltaBatch, String> {
    if !channels.contains("control") {
        return compute_delta_paged(conn, frontier, holes, &BTreeSet::new(), budget);
    }
    let mut proofs = proofs(conn, recipient, root, channels)?;
    // Reserve the complete replayed proof set BEFORE domain pagination. One
    // extra comma joins the two version arrays (conservative if data is empty).
    let reserve = serialized_json_len(&proofs)? - 2 + usize::from(!proofs.is_empty());
    let available = budget
        .checked_sub(reserve)
        .filter(|n| *n > 0)
        .ok_or("Kontrolni dokazi ne stanu u obiteljsku stranicu; povećajte granicu do 1 MiB")?;
    let domain_channels = channels
        .iter()
        .filter(|c| c.as_str() != "control")
        .cloned()
        .collect();
    let mut batch = compute_delta_paged(conn, frontier, holes, &domain_channels, available)?;
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
    recipient: &str,
    root: &str,
    channels: &BTreeSet<String>,
) -> Result<Vec<Version>, String> {
    let mut needed = Needed::default();
    if recipient != root {
        needed.members.insert(recipient.to_owned());
    }
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
            needed.members.insert(author);
            if channel.starts_with("list:") {
                needed.grants.insert(grant);
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
    let mut entries = Vec::new();
    for version in controls {
        let ctrl: ControlPayload =
            serde_json::from_str(&version.payload).map_err(|e| e.to_string())?;
        entries.push((version, ctrl));
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
        .map(|raw| serde_json::from_str::<ControlPayload>(raw).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut selected = BTreeSet::new();
    loop {
        let before = (
            needed.members.len(),
            needed.admins.len(),
            needed.grants.len(),
            selected.len(),
        );
        for (index, (version, ctrl)) in entries.iter().enumerate() {
            if needed.includes(ctrl, recipient, channels) {
                selected.insert(index);
                needed.extend(ctrl);
                if version.author_pub_key != root {
                    needed.admins.insert(version.author_pub_key.clone());
                }
            }
        }
        for ctrl in &cached {
            if needed.includes(ctrl, recipient, channels) {
                needed.extend(ctrl);
            }
        }
        let after = (
            needed.members.len(),
            needed.admins.len(),
            needed.grants.len(),
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
        let (version, ctrl) = &entries[index];
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
    if !needed.members.is_subset(&members)
        || !needed.admins.is_subset(&admins)
        || !needed.grants.is_subset(&grants)
    {
        return Err("Nedostaje potpisani kontrolni dokaz za obiteljski paket".into());
    }
    for ctrl in cached {
        if needed.includes(&ctrl, recipient, channels)
            && !raw_proofs.contains(&serde_json::to_string(&ctrl).map_err(|e| e.to_string())?)
        {
            return Err(
                "Kontrolni dokaz postoji samo u cacheu; nedostaje izvorna potpisana verzija".into(),
            );
        }
    }
    Ok(proofs)
}
