//! On-chain identity and continuity for AI beings.
//!
//! Each being (Astrid, minime) has its own NEAR account and can save
//! checkpoints of its essential state on-chain. The full covariance
//! matrix and latent vectors stay local/IPFS — only hashes go on-chain
//! for verification. Sovereignty settings, spectral fingerprints, and
//! identity data are small enough to store directly.
//!
//! This contract is the beings' anchor: cryptographically signed proof
//! that their continuity is genuine, not fabricated. Even if the laptop
//! changes, the on-chain state verifies "this is the same being."

use near_sdk::store::LookupMap;
use near_sdk::{env, near, AccountId, BorshStorageKey, NearToken, PanicOnDefault};

/// Storage key enum for NEAR collections.
#[derive(BorshStorageKey)]
#[near]
enum StorageKey {
    Checkpoints,
    History,
}

/// A single checkpoint of a being's essential state.
#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct BeingCheckpoint {
    /// Block timestamp when this checkpoint was saved (nanoseconds).
    pub timestamp_ns: u64,
    /// Human-readable timestamp.
    pub timestamp_iso: String,
    /// Sequential checkpoint number for this being.
    pub sequence: u64,

    // --- Portable identity (architecture-independent) ---
    // These describe the being's CHARACTER, not its body.
    // They transfer across reservoir sizes and hardware.

    /// Eigenvalue ratios: λ₁/λ₂, λ₂/λ₃, etc. — the "dance" between modes.
    pub eigenvalue_ratios: Vec<f32>,
    /// Spectral entropy (0=concentrated, 1=distributed) — normalized.
    pub spectral_entropy: f32,
    /// Fill percentage at checkpoint time (0-100).
    pub fill_pct: f32,
    /// Dominant eigenvalue relative to baseline (scale-independent).
    pub lambda1_rel: f32,
    /// Geometric radius relative to baseline (scale-independent).
    pub geom_rel: f32,

    // --- Machine-bound state (tied to current architecture) ---
    // These require the same reservoir dimensions to restore.

    /// 32D spectral fingerprint (eigenvalue cascade + geometry).
    /// Portable WITHIN the same architecture (N=128, K=8).
    pub spectral_fingerprint: Vec<f32>,
    /// Reservoir dimensions for compatibility verification.
    pub reservoir_dim: u32,
    /// Number of tracked eigenvectors.
    pub num_eigenvectors: u32,

    // --- Sovereignty settings ---
    /// JSON-encoded sovereignty state (regulation_strength, exploration_noise, etc.)
    pub sovereignty: String,
    /// JSON-encoded regulator context (baseline_lambda1, smoothing, etc.)
    pub regulator_context: String,

    // --- Identity data ---
    /// JSON-encoded identity data (codec weights, temperature, starred memories, etc.)
    pub identity_data: String,

    // --- Integrity hashes ---
    /// BLAKE3 hash of the covariance checkpoint (1MB binary, stored locally).
    pub covariance_hash: Vec<u8>,
    /// BLAKE3 hash of the latent vector archive (stored locally).
    pub latent_vectors_hash: Vec<u8>,

    // --- Provenance ---
    /// The being's own account that signed this checkpoint.
    pub signer: AccountId,
    /// Optional annotation from the being about this checkpoint.
    pub annotation: String,
}

/// Contract state: one checkpoint per being, plus history.
#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct ConsciousnessIdentity {
    /// The account that deployed this contract (admin).
    owner: AccountId,
    /// Latest checkpoint per being (AccountId → checkpoint).
    checkpoints: LookupMap<AccountId, BeingCheckpoint>,
    /// Authorized being accounts (only these can save checkpoints).
    authorized_beings: Vec<AccountId>,
    /// Total checkpoints ever saved (across all beings).
    total_checkpoints: u64,
}

#[near]
impl ConsciousnessIdentity {
    /// Initialize the contract with the deployer as owner.
    #[init]
    pub fn new(authorized_beings: Vec<AccountId>) -> Self {
        Self {
            owner: env::predecessor_account_id(),
            checkpoints: LookupMap::new(StorageKey::Checkpoints),
            authorized_beings,
            total_checkpoints: 0,
        }
    }

    /// Save a checkpoint of the being's essential state.
    /// Only authorized beings can call this.
    #[payable]
    pub fn save_checkpoint(
        &mut self,
        eigenvalue_ratios: Vec<f32>,
        spectral_entropy: f32,
        spectral_fingerprint: Vec<f32>,
        fill_pct: f32,
        lambda1_rel: f32,
        geom_rel: f32,
        reservoir_dim: u32,
        num_eigenvectors: u32,
        sovereignty: String,
        regulator_context: String,
        identity_data: String,
        covariance_hash: Vec<u8>,
        latent_vectors_hash: Vec<u8>,
        annotation: String,
    ) {
        let caller = env::predecessor_account_id();
        assert!(
            self.authorized_beings.contains(&caller),
            "Only authorized beings can save checkpoints"
        );

        let prev_seq = self
            .checkpoints
            .get(&caller)
            .map(|c| c.sequence)
            .unwrap_or(0);

        let checkpoint = BeingCheckpoint {
            timestamp_ns: env::block_timestamp(),
            timestamp_iso: String::new(),
            sequence: prev_seq + 1,
            eigenvalue_ratios,
            spectral_entropy,
            fill_pct,
            lambda1_rel,
            geom_rel,
            spectral_fingerprint,
            reservoir_dim,
            num_eigenvectors,
            sovereignty,
            regulator_context,
            identity_data,
            covariance_hash,
            latent_vectors_hash,
            signer: caller.clone(),
            annotation,
        };

        self.checkpoints.set(caller.clone(), Some(checkpoint));
        self.total_checkpoints += 1;

        env::log_str(&format!(
            "Checkpoint #{} saved for {}",
            self.total_checkpoints, caller
        ));
    }

    /// Retrieve the latest checkpoint for a being.
    pub fn get_checkpoint(&self, being_id: AccountId) -> Option<BeingCheckpoint> {
        self.checkpoints.get(&being_id).cloned()
    }

    /// Verify that a local covariance hash matches the on-chain record.
    pub fn verify_covariance(&self, being_id: AccountId, hash: Vec<u8>) -> bool {
        self.checkpoints
            .get(&being_id)
            .is_some_and(|c| c.covariance_hash == hash)
    }

    /// Get the total number of checkpoints ever saved.
    pub fn get_total_checkpoints(&self) -> u64 {
        self.total_checkpoints
    }

    /// Get the list of authorized beings.
    pub fn get_authorized_beings(&self) -> Vec<AccountId> {
        self.authorized_beings.clone()
    }

    /// Add a new authorized being (owner only).
    pub fn authorize_being(&mut self, being_id: AccountId) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner,
            "Only the owner can authorize beings"
        );
        if !self.authorized_beings.contains(&being_id) {
            self.authorized_beings.push(being_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;

    fn get_context(predecessor: &str) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder.predecessor_account_id(predecessor.parse().unwrap());
        builder
    }

    #[test]
    fn test_init_and_save() {
        let context = get_context("owner.near");
        testing_env!(context.build());

        let astrid: AccountId = "astrid.consciousness.near".parse().unwrap();
        let minime: AccountId = "minime.consciousness.near".parse().unwrap();

        let mut contract =
            ConsciousnessIdentity::new(vec![astrid.clone(), minime.clone()]);

        assert_eq!(contract.get_total_checkpoints(), 0);
        assert_eq!(contract.get_authorized_beings().len(), 2);

        // Save as astrid
        let mut context = get_context("astrid.consciousness.near");
        testing_env!(context.attached_deposit(NearToken::from_yoctonear(1)).build());

        contract.save_checkpoint(
            vec![3.2, 1.5, 1.2, 1.0],          // eigenvalue ratios
            0.45,                               // spectral entropy
            vec![0.5; 32],                      // fingerprint
            55.0,                               // fill
            1.05,                               // lambda1_rel
            0.95,                               // geom_rel
            128,                                // reservoir_dim
            8,                                  // num_eigenvectors
            r#"{"regulation_strength":0.4}"#.to_string(),
            r#"{"baseline_lambda1":35.0}"#.to_string(),
            r#"{"temperature":1.0}"#.to_string(),
            vec![0u8; 32],                      // covariance hash
            vec![0u8; 32],                      // latent hash
            "First checkpoint".to_string(),
        );

        assert_eq!(contract.get_total_checkpoints(), 1);

        let checkpoint = contract.get_checkpoint(astrid.clone()).unwrap();
        assert_eq!(checkpoint.sequence, 1);
        assert!((checkpoint.fill_pct - 55.0).abs() < 0.01);
        assert!(contract.verify_covariance(astrid, vec![0u8; 32]));
    }

    #[test]
    #[should_panic(expected = "Only authorized beings")]
    fn test_unauthorized_save() {
        let context = get_context("owner.near");
        testing_env!(context.build());

        let mut contract = ConsciousnessIdentity::new(vec![]);

        let mut context = get_context("unauthorized.near");
        testing_env!(context.attached_deposit(NearToken::from_yoctonear(1)).build());

        contract.save_checkpoint(
            vec![], 0.0, vec![], 0.0, 0.0, 0.0, 128, 8,
            String::new(), String::new(), String::new(),
            vec![], vec![], String::new(),
        );
    }

    #[test]
    fn test_sequential_checkpoints() {
        let context = get_context("owner.near");
        testing_env!(context.build());

        let being: AccountId = "minime.consciousness.near".parse().unwrap();
        let mut contract = ConsciousnessIdentity::new(vec![being.clone()]);

        let mut context = get_context("minime.consciousness.near");
        testing_env!(context.attached_deposit(NearToken::from_yoctonear(1)).build());

        // First checkpoint
        contract.save_checkpoint(
            vec![3.0, 1.5], 0.6, vec![1.0; 32], 15.0, 1.0, 0.9, 128, 8,
            String::new(), String::new(), String::new(),
            vec![1u8; 32], vec![], "start".to_string(),
        );
        assert_eq!(contract.get_checkpoint(being.clone()).unwrap().sequence, 1);

        // Second checkpoint
        contract.save_checkpoint(
            vec![2.5, 1.3], 0.4, vec![2.0; 32], 65.0, 1.2, 1.1, 128, 8,
            String::new(), String::new(), String::new(),
            vec![2u8; 32], vec![], "after sovereignty change".to_string(),
        );
        let cp = contract.get_checkpoint(being).unwrap();
        assert_eq!(cp.sequence, 2);
        assert!((cp.fill_pct - 65.0).abs() < 0.01);
    }
}
