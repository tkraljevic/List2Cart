//! Stable control identities and scoped, conservative revocation decisions.
use syncnet::trust::Ovlast;
use uuid::Uuid;

pub(crate) fn atomic(
    conn: &Connection,
    operation: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let name = format!("l2c_control_{}", Uuid::new_v4().simple());
    conn.execute_batch(&format!("SAVEPOINT {name}"))
        .map_err(|e| e.to_string())?;
    let result = operation().and_then(|()| {
        conn.execute_batch(&format!("RELEASE {name}"))
            .map_err(|e| e.to_string())
    });
    if let Err(error) = result {
        if let Err(rollback) = conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}")) {
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            return Err(format!(
                "{error}; neuspješno vraćanje kontrolnog zapisa: {rollback}"
            ));
        }
        return Err(error);
    }
    Ok(())
}
use rusqlite::{Connection, OptionalExtension, params};

/// Control history is retained by compaction. The cache remains the current
/// identity binding; historical proofs never replace it or restore revoked keys.
pub(crate) fn historical_membership(
    conn: &Connection,
    network: &ed25519_dalek::VerifyingKey,
    key: &str,
    at: i64,
) -> Result<Option<syncnet::trust::Membership>, String> {
    let mut newest: Option<syncnet::trust::Membership> = None;
    let mut seen_issued = std::collections::BTreeMap::<i64, Vec<u8>>::new();
    let mut seen_principal: Option<String> = None;
    for raw in control_history(conn, "membership", key)? {
        let crate::models::ControlPayload::Membership(member) =
            serde_json::from_str(&raw).map_err(|e| format!("Neispravan povijesni dokaz: {e}"))?
        else {
            continue;
        };
        if member.device_key != key {
            continue;
        }
        let signed = member.signed_bytes();
        if let Some(prev) = seen_issued.get(&member.issued_at) {
            if *prev != signed {
                return Err("Dvosmisleno povijesno članstvo".into());
            }
        } else {
            seen_issued.insert(member.issued_at, signed);
        }
        if let Some(ref principal) = seen_principal {
            if *principal != member.principal_id {
                return Err("Povijesno članstvo ne smije promijeniti principal".into());
            }
        } else {
            seen_principal = Some(member.principal_id.clone());
        }
        if member.issued_at <= at
            && newest
                .as_ref()
                .is_none_or(|old| member.issued_at > old.issued_at)
        {
            newest = Some(member);
        }
    }
    let Some(member) = newest else {
        return Ok(None);
    };
    // Select the latest issuance first: expiry or a downgrade must not fall
    // through to an older, more permissive certificate.
    let public = syncnet::identity::parse_public_key(key).map_err(|e| e.to_string())?;
    if member.verify(network, &public, at).is_err()
        || check_delegation_at(conn, network, member.admitter.as_ref(), member.issued_at).is_err()
        || revoked(conn, "membership", key, &member.device_id)?
    {
        return Ok(None);
    }
    Ok(Some(member))
}

pub(crate) fn historical_admin(
    conn: &Connection,
    network: &ed25519_dalek::VerifyingKey,
    key: &str,
    at: i64,
) -> Result<bool, String> {
    if revoked(conn, "ovlast", key, "")? {
        return Ok(false);
    }
    let mut newest: Option<Ovlast> = None;
    let mut seen_issued = std::collections::BTreeMap::<i64, Vec<u8>>::new();
    for raw in control_history(conn, "ovlast", key)? {
        let crate::models::ControlPayload::Ovlast(authority) =
            serde_json::from_str(&raw).map_err(|e| format!("Neispravan povijesni dokaz: {e}"))?
        else {
            continue;
        };
        if authority.device_key != key {
            continue;
        }
        let signed = authority.signed_bytes();
        if let Some(prev) = seen_issued.get(&authority.issued_at) {
            if *prev != signed {
                return Err("Dvosmislena povijesna ovlast".into());
            }
        } else {
            seen_issued.insert(authority.issued_at, signed);
        }
        if authority.issued_at <= at
            && newest
                .as_ref()
                .is_none_or(|old| authority.issued_at > old.issued_at)
        {
            newest = Some(authority);
        }
    }
    Ok(newest.is_some_and(|authority| authority.verify(network, at).is_ok()))
}

fn control_history(conn: &Connection, kind: &str, key: &str) -> Result<Vec<String>, String> {
    let mut statement = conn.prepare(
        "SELECT raw_json FROM sync_trust_records WHERE kind=?1 AND subject_key=?2
         UNION SELECT payload FROM record_versions WHERE channel='control' AND signature!=''
           AND legacy_imported=0 AND CASE WHEN json_valid(payload) THEN
             json_extract(payload,'$.control_kind')=?1 AND json_extract(payload,'$.data.deviceKey')=?2
           ELSE 0 END"
    ).map_err(|e|e.to_string())?;
    statement
        .query_map(params![kind, key], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub(crate) struct ControlIdentity<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub subject: &'a str,
    pub target: &'a str,
    pub issued: i64,
    pub raw: &'a str,
}

/// False means an older signed proof may be retained in the ledger, but cannot
/// replace the current authority. Grant IDs and principal bindings are immutable.
pub(crate) fn may_store(conn: &Connection, value: ControlIdentity<'_>) -> Result<bool, String> {
    if matches!(value.kind, "membership" | "ovlast") {
        for raw in control_history(conn, value.kind, value.subject)? {
            let previous: crate::models::ControlPayload = serde_json::from_str(&raw)
                .map_err(|e| format!("Neispravan povijesni dokaz: {e}"))?;
            let (issued, same_target) = match previous {
                crate::models::ControlPayload::Membership(m) => {
                    (m.issued_at, m.principal_id == value.target)
                }
                crate::models::ControlPayload::Ovlast(o) => {
                    (o.issued_at, o.device_id == value.target)
                }
                _ => continue,
            };
            if !same_target {
                return Err(
                    "Kontrolni identitet ne smije promijeniti vrstu, ključ ili cilj".into(),
                );
            }
            if issued == value.issued && raw != value.raw {
                return Err("Dvosmisleno povijesno izdavanje kontrolnog zapisa".into());
            }
        }
    }
    let old:Option<(String,String,String,i64,String)>=conn.query_row("SELECT kind,subject_key,target_id,issued_at,raw_json FROM sync_trust_records WHERE record_id=?1",[value.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?;
    let Some((kind, subject, target, issued, raw)) = old else {
        return Ok(true);
    };
    if kind != value.kind || subject != value.subject || target != value.target {
        return Err("Kontrolni identitet ne smije promijeniti vrstu, ključ ili cilj".into());
    }
    if raw == value.raw {
        return Ok(true);
    }
    if kind == "list_grant" || kind == "revocation" {
        return Err("Isti ID ovlasti ne smije imati drugi sadržaj; izdajte novi grant".into());
    }
    if value.issued < issued {
        return Ok(false);
    }
    if value.issued == issued {
        return Err("Dvosmisleno izdavanje kontrolnog zapisa u isto vrijeme".into());
    }
    Ok(true)
}

pub(crate) fn revoked(
    conn: &Connection,
    kind: &str,
    subject: &str,
    id: &str,
) -> Result<bool, String> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_trust_records WHERE kind='revocation' AND subject_key=?1 AND (
        permission IN ('membership','legacy_key') OR (permission='list_grant' AND ?2='list_grant' AND target_id=?3) OR (permission='ovlast' AND ?2='ovlast')))",params![subject,kind,id],|r|r.get(0)).map_err(|e|e.to_string())
}

pub(crate) fn check_delegation(conn: &Connection, admitter: Option<&Ovlast>) -> Result<(), String> {
    if let Some(issuer) = admitter
        && revoked(conn, "ovlast", &issuer.device_key, &issuer.device_id)?
    {
        return Err("Izdavateljeva administratorska ovlast je opozvana".into());
    }
    Ok(())
}

pub(crate) fn check_delegation_at(
    conn: &Connection,
    network: &ed25519_dalek::VerifyingKey,
    admitter: Option<&Ovlast>,
    issued: i64,
) -> Result<(), String> {
    check_delegation(conn, admitter)?;
    let Some(embedded) = admitter else {
        return Ok(());
    };
    let mut effective: Option<Ovlast> = None;
    let mut seen_issued = std::collections::BTreeMap::<i64, Vec<u8>>::new();
    for raw in control_history(conn, "ovlast", &embedded.device_key)? {
        let crate::models::ControlPayload::Ovlast(proof) =
            serde_json::from_str(&raw).map_err(|e| e.to_string())?
        else {
            continue;
        };
        let signed = proof.signed_bytes();
        if let Some(prev) = seen_issued.get(&proof.issued_at) {
            if *prev != signed {
                return Err("Dvosmislena povijesna ovlast".into());
            }
        } else {
            seen_issued.insert(proof.issued_at, signed);
        }
        if proof.issued_at <= issued
            && effective
                .as_ref()
                .is_none_or(|old| proof.issued_at > old.issued_at)
        {
            effective = Some(proof);
        }
    }
    if let Some(proof) = effective
        && (proof.verify(network, issued).is_err()
            || proof.signed_bytes() != embedded.signed_bytes())
    {
        return Err("Ugrađena delegacija nije tada važeća administratorska ovlast".into());
    }
    Ok(())
}

pub(crate) fn apply_revocation_flags(conn: &Connection, reset: bool) -> Result<(), String> {
    if reset {
        conn.execute(
            "UPDATE sync_trust_records SET revoked=0 WHERE kind!='revocation'",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    conn.execute("UPDATE sync_trust_records AS t SET revoked=1 WHERE t.kind!='revocation' AND EXISTS(
        SELECT 1 FROM sync_trust_records r WHERE r.kind='revocation' AND (
          (r.permission IN ('membership','legacy_key') AND r.subject_key=t.subject_key)
          OR (r.permission='list_grant' AND t.kind='list_grant' AND r.subject_key=t.subject_key AND r.target_id=t.record_id)
          OR (r.permission IN ('ovlast','membership','legacy_key') AND ((t.kind='ovlast' AND r.subject_key=t.subject_key)
            OR (CASE WHEN json_valid(t.raw_json) THEN json_extract(t.raw_json,'$.data.admitter.deviceKey') END=r.subject_key)))
        ))",[]).map_err(|e|format!("Greska primjene opsega opoziva: {e}"))?;
    Ok(())
}
