use sha2::{Digest, Sha256};

pub const SAS_NONCE_LEN: usize = 32;

pub fn compute_sas_commit(protocol: &str, nonce: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(protocol.as_bytes());
    h.update(b"-pair-commit-v2|");
    h.update(nonce);
    h.finalize().into()
}

pub fn verify_sas_commit(protocol: &str, revealed_nonce: &[u8], expected_commit: &[u8]) -> bool {
    if revealed_nonce.len() != SAS_NONCE_LEN || expected_commit.len() != 32 {
        return false;
    }
    let actual = compute_sas_commit(protocol, revealed_nonce);
    // Constant-time comparison
    let mut diff = 0u8;
    for (a, b) in actual.iter().zip(expected_commit.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn compute_sas_code(
    protocol: &str,
    pub_a: &[u8; 32],
    pub_b: &[u8; 32],
    caller_nonce: &[u8; SAS_NONCE_LEN],
    listener_nonce: &[u8; SAS_NONCE_LEN],
) -> String {
    let (x, y) = if pub_a <= pub_b {
        (pub_a, pub_b)
    } else {
        (pub_b, pub_a)
    };
    let mut h = Sha256::new();
    h.update(protocol.as_bytes());
    h.update(b"-pair-v2|");
    h.update(x);
    h.update(y);
    h.update(caller_nonce);
    h.update(listener_nonce);
    let sum = h.finalize();
    let n = u32::from_be_bytes([sum[0], sum[1], sum[2], sum[3]]) % 1_000_000;
    format!("{n:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sas_code_is_order_independent_for_keys_and_order_dependent_for_nonces() {
        let a = [0x10u8; 32];
        let b = [0x20u8; 32];
        let nc = [0x01u8; 32];
        let nl = [0x02u8; 32];

        let s1 = compute_sas_code("list2cart", &a, &b, &nc, &nl);
        let s2 = compute_sas_code("list2cart", &b, &a, &nc, &nl);
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 6);

        let s_swapped_nonces = compute_sas_code("list2cart", &a, &b, &nl, &nc);
        assert_ne!(s1, s_swapped_nonces);

        let s_other_proto = compute_sas_code("list2cart-sim", &a, &b, &nc, &nl);
        assert_ne!(s1, s_other_proto);
    }

    #[test]
    fn sas_commit_reveal_verifies_and_rejects_tampered_nonce() {
        let nonce = [0xABu8; SAS_NONCE_LEN];
        let commit = compute_sas_commit("list2cart", &nonce);
        assert!(verify_sas_commit("list2cart", &nonce, &commit));

        let mut bad = nonce;
        bad[0] ^= 1;
        assert!(!verify_sas_commit("list2cart", &bad, &commit));
        assert!(!verify_sas_commit("list2cart", &[0u8; 16], &commit));
    }
}
