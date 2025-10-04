#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Vec, Bytes,
};

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Admin,
    Threshold,
    Signers,
    NextId,
    Transaction(u64),
    Approvals(u64),
    Signer(Address),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[contracttype]
pub enum TransactionStatus {
    Pending,
    Executed,
    Rejected,
    Cancelled,
}

#[derive(Clone)]
#[contracttype]
pub struct Transaction {
    pub id: u64,
    pub to: Address,
    pub amount: i128,
    pub data: Bytes,
    pub status: TransactionStatus,
    pub proposed_by: Address,
    pub created_at: u64,
}

#[contract]
pub struct MultiSigContract;

#[contractimpl]
impl MultiSigContract {
    
    pub fn initialize(env: Env, admin: Address, signers: Vec<Address>, threshold: u32) {
        assert!(!env.storage().persistent().has(&DataKey::Admin), "Contract already initialized");
        let signers_len: u32 = signers.len().try_into().unwrap();
        assert!(threshold > 0 && threshold <= signers_len, "Invalid threshold");
        assert!(!signers.is_empty(), "At least one signer is required");

        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Threshold, &threshold);

        for signer in signers.iter() {
            env.storage().persistent().set(&DataKey::Signer(signer.clone()), &true);
            Self::update_signers_list(&env, &signer, true);
        }

        env.storage().persistent().set(&DataKey::NextId, &1u64);
    }

    // --- Authentication helpers ---
    fn only_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        assert!(*caller == admin, "Caller is not the admin");
    }

    fn only_signer(env: &Env, caller: &Address) {
        caller.require_auth();
        assert!(
            env.storage().persistent().has(&DataKey::Signer(caller.clone())),
            "Caller is not a signer"
        );
    }

    // --- Admin functions ---
    pub fn add_signer(env: Env, caller: Address, signer: Address) {
        Self::only_admin(&env, &caller);
        assert!(
            !env.storage().persistent().has(&DataKey::Signer(signer.clone())),
            "Signer already exists"
        );

        env.storage().persistent().set(&DataKey::Signer(signer.clone()), &true);
        Self::update_signers_list(&env, &signer, true);
    }

    pub fn remove_signer(env: Env, caller: Address, signer: Address) {
        Self::only_admin(&env, &caller);
        let threshold: u32 = env.storage().persistent().get(&DataKey::Threshold).unwrap();
        let current_signers = Self::get_signers(&env);
        let current_len = current_signers.len() as i128;
        let threshold_i128 = threshold as i128;
        assert!(
            current_len > threshold_i128,
            "Cannot remove signer: would go below threshold"
        );

        env.storage().persistent().remove(&DataKey::Signer(signer.clone()));
        Self::update_signers_list(&env, &signer, false);
    }

    pub fn update_threshold(env: Env, caller: Address, new_threshold: u32) {
        Self::only_admin(&env, &caller);
        let current_signers = Self::get_signers(&env);
        let current_len = current_signers.len() as u32;
        assert!(
            new_threshold > 0 && new_threshold <= current_len,
            "Invalid threshold"
        );

        env.storage().persistent().set(&DataKey::Threshold, &new_threshold);
    }

    // --- Transaction functions ---
    pub fn propose_transaction(
        env: Env,
        caller: Address,
        to: Address,
        amount: i128,
        data: Bytes,
    ) -> u64 {
        Self::only_signer(&env, &caller);

        let tx_id: u64 = env.storage().persistent().get(&DataKey::NextId).unwrap();
        env.storage().persistent().set(&DataKey::NextId, &(tx_id + 1));

        let tx = Transaction {
            id: tx_id,
            to,
            amount,
            data,
            status: TransactionStatus::Pending,
            proposed_by: caller.clone(),
            created_at: env.ledger().timestamp(),
        };

        env.storage().persistent().set(&DataKey::Transaction(tx_id), &tx);

        Self::self_approve(&env, &caller, tx_id);

        tx_id
    }

    pub fn approve_transaction(env: Env, caller: Address, tx_id: u64) {
        Self::only_signer(&env, &caller);

        let mut tx: Transaction = env
            .storage()
            .persistent()
            .get(&DataKey::Transaction(tx_id))
            .unwrap_or_else(|| panic!("Transaction not found"));

        assert!(
            tx.status == TransactionStatus::Pending,
            "Transaction is not pending"
        );

        Self::self_approve(&env, &caller, tx_id);

        let approvals = Self::get_approvals(&env, tx_id);
        let threshold: u32 = env.storage().persistent().get(&DataKey::Threshold).unwrap();

        let threshold: u32 = env.storage().persistent().get(&DataKey::Threshold).unwrap();
        if approvals.len() as u32 >= threshold {
            Self::self_execute(&env, &mut tx);
        }
    }

    fn self_execute(env: &Env, tx: &mut Transaction) {
        tx.status = TransactionStatus::Executed;
        // Clone the transaction to avoid mutable reference issues
        let tx_clone = tx.clone();
        env.storage().persistent().set(&DataKey::Transaction(tx.id), &tx_clone);

    }

    pub fn get_signers(env: &Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Signers)
            .unwrap_or_else(|| Vec::new(env))
    }

    fn update_signers_list(env: &Env, signer: &Address, is_add: bool) {
        let mut signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Signers)
            .unwrap_or_else(|| Vec::new(env));

        if is_add {
            if !signers.contains(signer) {
                signers.push_back(signer.clone());
            }
        } else {
            let mut new_signers = Vec::new(env);
            for s in signers.iter() {
                if &s != signer {
                    new_signers.push_back(s);
                }
            }
            env.storage().persistent().set(&DataKey::Signers, &new_signers);
        }

        env.storage().persistent().set(&DataKey::Signers, &signers);
    }

    pub fn get_approvals(env: &Env, tx_id: u64) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Approvals(tx_id))
            .unwrap_or_else(|| Vec::new(env))
    }

    pub fn get_transaction(env: Env, tx_id: u64) -> Option<Transaction> {
        env.storage().persistent().get(&DataKey::Transaction(tx_id))
    }

    fn self_approve(env: &Env, caller: &Address, tx_id: u64) {
        let mut approvals: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Approvals(tx_id))
            .unwrap_or_else(|| Vec::new(env));

        let mut found = false;
        for addr in approvals.iter() {
            if &addr == caller {
                found = true;
                break;
            }
        }

        if !found {
            approvals.push_back(caller.clone());
            env.storage().persistent().set(&DataKey::Approvals(tx_id), &approvals);
        }
    }
}
