use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use syncnet::identity::public_key_to_base64;
use syncnet::trust::{
    NetworkKey, Ovlast, TrustError, Validity, verify_admitter_chain, verify_ed25519_b64,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    OwnDevice,
    FamilyMember,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListPermission {
    Read,
    Write,
}

/// Application-level signed record binding a List2Cart device to its role and principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberRoleRecord {
    pub network: String,
    pub device_id: String,
    pub device_key: String,
    pub role: MemberRole,
    pub principal_id: String,
    pub issued_by: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitter: Option<Ovlast>,
}

impl MemberRoleRecord {
    pub fn signed_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Canonical<'a> {
            v: &'a str,
            network: &'a str,
            device_id: &'a str,
            device_key: &'a str,
            role: MemberRole,
            principal_id: &'a str,
            issued_by: &'a str,
            issued_at: i64,
            expires_at: i64,
            ovlast: &'a str,
        }
        let ovlast_sig = self
            .admitter
            .as_ref()
            .map(|o| o.signature.as_str())
            .unwrap_or("");
        serde_json::to_vec(&Canonical {
            v: "list2cart-member-role/1",
            network: &self.network,
            device_id: &self.device_id,
            device_key: &self.device_key,
            role: self.role,
            principal_id: &self.principal_id,
            issued_by: &self.issued_by,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
            ovlast: ovlast_sig,
        })
        .expect("canonical member role serializes")
    }

    pub fn admit_as(
        admitter_sk: &SigningKey,
        ovlast: Ovlast,
        device_id: &str,
        device_key: &VerifyingKey,
        role: MemberRole,
        principal_id: &str,
        validity: Validity,
    ) -> Result<Self, TrustError> {
        let issued_at = validity.issued_at;
        let admitter_pub_b64 = public_key_to_base64(admitter_sk.verifying_key().as_bytes());
        if admitter_pub_b64 != ovlast.device_key {
            return Err(TrustError::UnauthorizedSigner);
        }
        if issued_at < ovlast.issued_at || (ovlast.expires_at > 0 && issued_at > ovlast.expires_at)
        {
            return Err(TrustError::Expired);
        }
        let mut exp = validity.expires_at()?;
        if ovlast.expires_at > 0 && exp > ovlast.expires_at {
            exp = ovlast.expires_at;
        }
        let issued_by = ovlast.device_id.clone();
        let mut rec = Self {
            network: ovlast.network.clone(),
            device_id: device_id.to_owned(),
            device_key: public_key_to_base64(device_key.as_bytes()),
            role,
            principal_id: principal_id.to_owned(),
            issued_by,
            issued_at,
            expires_at: exp,
            signature: String::new(),
            admitter: Some(ovlast),
        };
        let sig = admitter_sk.sign(&rec.signed_bytes());
        rec.signature = STANDARD.encode(sig.to_bytes());
        Ok(rec)
    }

    pub fn verify(
        &self,
        network_pub: &VerifyingKey,
        device_key: &VerifyingKey,
        now_unix: i64,
    ) -> Result<(), TrustError> {
        if self.network != public_key_to_base64(network_pub.as_bytes()) {
            return Err(TrustError::OtherNetwork);
        }
        if self.device_key != public_key_to_base64(device_key.as_bytes()) {
            return Err(TrustError::KeyMismatch);
        }
        let signer = verify_admitter_chain(
            network_pub,
            self.admitter.as_ref(),
            &self.issued_by,
            self.issued_at,
            self.expires_at,
            now_unix,
        )?;
        verify_ed25519_b64(&signer, &self.signed_bytes(), &self.signature)?;
        if self.expires_at > 0 && now_unix > self.expires_at {
            return Err(TrustError::Expired);
        }
        Ok(())
    }
}

/// Issues a root-signed `MemberRoleRecord` using the network signing key.
pub fn issue_member_role(
    network: &NetworkKey,
    device_id: &str,
    device_key: &VerifyingKey,
    role: MemberRole,
    principal_id: &str,
    issued_by: &str,
    validity: Validity,
) -> Result<MemberRoleRecord, TrustError> {
    let sk = network.signing_key()?;
    let mut rec = MemberRoleRecord {
        network: network.public_b64(),
        device_id: device_id.to_owned(),
        device_key: public_key_to_base64(device_key.as_bytes()),
        role,
        principal_id: principal_id.to_owned(),
        issued_by: issued_by.to_owned(),
        issued_at: validity.issued_at,
        expires_at: validity.expires_at()?,
        signature: String::new(),
        admitter: None,
    };
    let sig = sk.sign(&rec.signed_bytes());
    rec.signature = STANDARD.encode(sig.to_bytes());
    Ok(rec)
}

/// Application-level signed grant for sharing a specific shopping list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListGrant {
    pub network: String,
    pub grant_id: String,
    pub list_sync_id: String,
    pub recipient_pub_key: String,
    pub permission: ListPermission,
    pub issued_by: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitter: Option<Ovlast>,
}

impl ListGrant {
    pub fn signed_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Canonical<'a> {
            v: &'a str,
            network: &'a str,
            grant_id: &'a str,
            list_sync_id: &'a str,
            recipient_pub_key: &'a str,
            permission: ListPermission,
            issued_by: &'a str,
            issued_at: i64,
            expires_at: i64,
            ovlast: &'a str,
        }
        let ovlast_sig = self
            .admitter
            .as_ref()
            .map(|o| o.signature.as_str())
            .unwrap_or("");
        serde_json::to_vec(&Canonical {
            v: "listgrant/1",
            network: &self.network,
            grant_id: &self.grant_id,
            list_sync_id: &self.list_sync_id,
            recipient_pub_key: &self.recipient_pub_key,
            permission: self.permission,
            issued_by: &self.issued_by,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
            ovlast: ovlast_sig,
        })
        .expect("canonical listgrant serializes")
    }

    pub fn verify(
        &self,
        network_pub: &VerifyingKey,
        recipient_key: &VerifyingKey,
        now_unix: i64,
    ) -> Result<(), TrustError> {
        if self.network != public_key_to_base64(network_pub.as_bytes()) {
            return Err(TrustError::OtherNetwork);
        }
        if self.recipient_pub_key != public_key_to_base64(recipient_key.as_bytes()) {
            return Err(TrustError::KeyMismatch);
        }
        let signer = verify_admitter_chain(
            network_pub,
            self.admitter.as_ref(),
            &self.issued_by,
            self.issued_at,
            self.expires_at,
            now_unix,
        )?;
        verify_ed25519_b64(&signer, &self.signed_bytes(), &self.signature)?;
        if self.expires_at > 0 && now_unix > self.expires_at {
            return Err(TrustError::Expired);
        }
        Ok(())
    }
}

/// Issues a root-signed `ListGrant` using the network signing key.
pub fn issue_list_grant(
    network: &NetworkKey,
    grant_id: &str,
    list_sync_id: &str,
    recipient_key: &VerifyingKey,
    permission: ListPermission,
    issued_by: &str,
    validity: Validity,
) -> Result<ListGrant, TrustError> {
    let sk = network.signing_key()?;
    let mut g = ListGrant {
        network: network.public_b64(),
        grant_id: grant_id.to_owned(),
        list_sync_id: list_sync_id.to_owned(),
        recipient_pub_key: public_key_to_base64(recipient_key.as_bytes()),
        permission,
        issued_by: issued_by.to_owned(),
        issued_at: validity.issued_at,
        expires_at: validity.expires_at()?,
        signature: String::new(),
        admitter: None,
    };
    let sig = sk.sign(&g.signed_bytes());
    g.signature = STANDARD.encode(sig.to_bytes());
    Ok(g)
}
