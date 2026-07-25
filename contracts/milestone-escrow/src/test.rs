#![cfg(test)]
use super::*;
use crate::Error::NotFunded;
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, testutils::EnvTestConfig,
    testutils::Events, testutils::Ledger, vec, Address, Env, FromVal, IntoVal, Symbol, Val,
};

#[contracttype]
enum ReentrantTokenDataKey {
    Reentered,
}

#[contract]
pub struct ReentrantToken;

#[contractimpl]
impl ReentrantToken {
    pub fn transfer(env: Env, from: Address, to: Address, _amount: i128) {
        if env.storage().instance().has(&ReentrantTokenDataKey::Reentered) {
            return;
        }

        env.storage()
            .instance()
            .set(&ReentrantTokenDataKey::Reentered, &true);

        let escrow = MilestoneEscrowClient::new(&env, &to);
        let _ = escrow.try_fund(&from);
    }

    pub fn callback_attempted(env: Env) -> bool {
        env.storage().instance().has(&ReentrantTokenDataKey::Reentered)
    }
}

fn setup_funded_escrow(
    env: &Env,
    milestone_amounts: soroban_sdk::Vec<i128>,
) -> (
    Address,
    Address,
    Address,
    Address,
    Address,
    soroban_sdk::Address,
    MilestoneEscrowClient<'_>,
) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let arbiter_addr = Address::generate(env);
    let admin_addr = Address::generate(env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(env, &token_contract_id);
    let total: i128 = milestone_amounts.iter().sum();
    token_admin.mint(&client_addr, &total);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(env, &contract_id);

    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &milestone_amounts,
    );
    client.fund(&client_addr);

    (
        client_addr,
        freelancer_addr,
        arbiter_addr,
        admin_addr,
        token_contract_id,
        contract_id,
        client,
    )
}

#[test]
fn test_full_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 3_000_i128, 7_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    assert_eq!(token.balance(&client_addr), 10_000);

    client.fund(&client_addr);
    assert_eq!(token.balance(&client_addr), 0);
    assert_eq!(token.balance(&contract_id), 10_000);

    client.mark_delivered(&freelancer_addr, &0u32);

    client.approve_milestone(&client_addr, &0u32);
    assert_eq!(token.balance(&freelancer_addr), 3_000);
    assert_eq!(token.balance(&contract_id), 7_000);

    client.mark_delivered(&freelancer_addr, &1u32);
    client.approve_milestone(&client_addr, &1u32);
    assert_eq!(token.balance(&freelancer_addr), 10_000);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn test_dispute_release_to_freelancer() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &5_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 5_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);
    client.raise_dispute(&client_addr, &0u32);
    client.resolve_dispute(&arbiter_addr, &0u32, &true);

    assert_eq!(token.balance(&freelancer_addr), 5_000);
}

#[test]
fn test_dispute_refund_to_client() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &5_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 5_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.raise_dispute(&client_addr, &0u32);
    client.resolve_dispute(&arbiter_addr, &0u32, &false);

    assert_eq!(token.balance(&client_addr), 5_000);
}

#[test]
fn test_double_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    assert!(result.is_err());
}

#[test]
fn test_unauthorized_fund_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let bad_actor = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_fund(&bad_actor);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_invalid_milestone_index_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    let result = client.try_mark_delivered(&freelancer_addr, &1u32);
    assert_eq!(result, Err(Ok(Error::InvalidMilestone)));
}

#[test]
fn test_mark_delivered_zero_address_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    let zero_account = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    let result = client.try_mark_delivered(&zero_account, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidAddress)));
}

#[test]
fn test_mark_delivered_invalid_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    // Mock storage to change milestone amount to 0
    let milestone = Milestone {
        amount: 0,
        released_amount: 0,
        status: MilestoneStatus::Pending,
        delivered_at: 0,
    };
    env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::Milestone(0u32), &milestone);
    });

    let result = client.try_mark_delivered(&freelancer_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn test_mark_delivered_wrong_status_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    // Double-deliver must return the exact InvalidStatus error.
    let result = client.try_mark_delivered(&freelancer_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

/// mark_delivered on a milestone that has already been fully Released
/// (client approved) must return InvalidStatus.
#[test]
fn test_mark_delivered_after_released_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, freelancer_addr, _, _, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_milestone(&client_addr, &0u32);

    let result = client.try_mark_delivered(&freelancer_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

/// mark_delivered on a Disputed milestone must return InvalidStatus.
#[test]
fn test_mark_delivered_after_disputed_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, freelancer_addr, _, _, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.mark_delivered(&freelancer_addr, &0u32);
    client.raise_dispute(&client_addr, &0u32);

    let result = client.try_mark_delivered(&freelancer_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

/// mark_delivered on a Refunded milestone must return InvalidStatus.
#[test]
fn test_mark_delivered_after_refunded_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, freelancer_addr, arbiter_addr, _, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.mark_delivered(&freelancer_addr, &0u32);
    client.raise_dispute(&client_addr, &0u32);
    client.resolve_dispute(&arbiter_addr, &0u32, &false);

    let result = client.try_mark_delivered(&freelancer_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

/// mark_delivered on a PartiallyReleased milestone must return InvalidStatus.
#[test]
fn test_mark_delivered_after_partially_released_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, freelancer_addr, _, _, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_partial(&client_addr, &0u32, &400_i128);

    let result = client.try_mark_delivered(&freelancer_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

#[test]
fn test_approve_milestone_zero_account_address_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin_addr = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &vec![&env, 1_000_i128],
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let zero_account = Address::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    );
    let result = client.try_approve_milestone(&zero_account, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidAddress)));
}

#[test]
fn test_approve_milestone_zero_contract_address_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin_addr = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &vec![&env, 1_000_i128],
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let zero_contract = Address::from_str(
        &env,
        "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
    );
    let result = client.try_approve_milestone(&zero_contract, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidAddress)));
}

#[test]
fn test_approve_milestone_wrong_status_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    let result = client.try_approve_milestone(&client_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

#[test]
fn test_approve_milestone_invalid_index_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    // Try to approve milestone at non-existent index
    let result = client.try_approve_milestone(&client_addr, &1u32);
    assert_eq!(result, Err(Ok(Error::InvalidMilestone)));
}

#[test]
fn test_approve_milestone_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    // Milestone with zero amount should be rejected before state is written.
    let amounts = vec![&env, 0_i128];
    let result = client.try_initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

fn test_raise_dispute_unauthorized_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let bad_actor = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    let result = client.try_raise_dispute(&bad_actor, &0u32);
    assert!(result.is_err());
}

#[test]
fn test_raise_dispute_wrong_status_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_milestone(&client_addr, &0u32);

    let result = client.try_raise_dispute(&client_addr, &0u32);
    assert!(result.is_err());
}

#[test]
fn test_resolve_dispute_unauthorized_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let bad_actor = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.raise_dispute(&client_addr, &0u32);

    let result = client.try_resolve_dispute(&bad_actor, &0u32, &true);
    assert!(result.is_err());
}

#[test]
fn test_resolve_dispute_wrong_status_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    let result = client.try_resolve_dispute(&arbiter_addr, &0u32, &true);
    assert!(result.is_err());
}

#[test]
fn test_fund_before_initialized_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let result = client.try_fund(&client_addr);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn test_double_fund_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &2_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    let result = client.try_fund(&client_addr);
    assert_eq!(result, Err(Ok(Error::AlreadyFunded)));
}

#[test]
fn test_fund_reentrancy_guard_blocks_callback_fund() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env.register(ReentrantToken, ());
    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);
    let token = ReentrantTokenClient::new(&env, &token_contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    client.fund(&client_addr);

    assert!(token.callback_attempted());
    let job = client.get_job();
    assert!(job.funded);
    assert_eq!(job.milestones.len(), 1);

    let result = client.try_fund(&client_addr);
    assert_eq!(result, Err(Ok(Error::AlreadyFunded)));
}

#[test]
fn test_fund_emits_structured_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &3_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128, 2_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    let fund_topic_val: Val = symbol_short!("fund").into_val(&env);
    let mut fund_events = 0u32;
    for event in env.events().all().iter() {
        if let Some(topic) = event.1.get(0) {
            if topic.get_payload() == fund_topic_val.get_payload() {
                fund_events += 1;
                assert_eq!(event.1.len(), 1);
                assert_eq!(
                    FundedEvent::from_val(&env, &event.2),
                    FundedEvent {
                        contract_id: contract_id.clone(),
                        client: client_addr.clone(),
                        freelancer: freelancer_addr.clone(),
                        arbiter: arbiter_addr.clone(),
                        token: token_contract_id.clone(),
                        total_amount: 3_000,
                        milestone_count: 2,
                        auto_release_seconds: 604800,
                        funded: true,
                    }
                );
            }
        }
    }

    assert_eq!(fund_events, 1);
}

#[test]
fn test_failed_fund_does_not_emit_fund_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let wrong_client = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_fund(&wrong_client);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));

    let fund_topic_val: Val = symbol_short!("fund").into_val(&env);
    let fund_events = env.events().all().iter().fold(0u32, |acc, event| {
        if let Some(topic) = event.1.get(0) {
            if topic.get_payload() == fund_topic_val.get_payload() {
                return acc + 1;
            }
        }
        acc
    });
    assert_eq!(fund_events, 0);
}

#[test]
fn test_fund_before_initialize_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let result = client.try_fund(&client_addr);
    assert_eq!(result, Err(Ok(Error::NotInitialized)));
}

#[test]
fn test_fund_rejects_contract_address() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let result = client.try_fund(&contract_id);
    assert_eq!(result, Err(Ok(Error::InvalidAddress)));
}

#[test]
fn test_fund_rejects_wrong_client() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let wrong_client = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_fund(&wrong_client);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_fund_fails_without_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_fund(&client_addr);
    assert!(result.is_err());
}

#[test]
fn test_fund_uses_cached_total_for_many_milestones() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);

    let mut milestone_amounts = vec![&env];
    let mut total = 0_i128;
    for _ in 0..100u32 {
        milestone_amounts.push_back(1_i128);
        total += 1;
    }
    token_admin.mint(&client_addr, &total);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &milestone_amounts,
    );

    client.fund(&client_addr);

    assert_eq!(token.balance(&client_addr), 0);
    assert_eq!(token.balance(&contract_id), total);
    let job = client.get_job();
    assert!(job.funded);
    assert_eq!(job.milestones.len(), 100);
}

#[test]
fn test_fund_rejects_missing_milestone_index() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &3_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128, 2_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    env.as_contract(&contract_id, || {
        env.storage().persistent().remove(&DataKey::Milestone(1u32));
    });

    let result = client.try_fund(&client_addr);
    assert_eq!(result, Err(Ok(Error::InvalidMilestone)));
    assert_eq!(token.balance(&client_addr), 3_000);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn test_fund_rejects_zero_milestone_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    env.as_contract(&contract_id, || {
        let mut milestone: Milestone = env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(0u32))
            .unwrap();
        milestone.amount = 0;
        env.storage()
            .persistent()
            .set(&DataKey::Milestone(0u32), &milestone);
    });

    let result = client.try_fund(&client_addr);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(token.balance(&client_addr), 1_000);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn test_fund_rejects_negative_milestone_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &1_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    env.as_contract(&contract_id, || {
        let mut milestone: Milestone = env
            .storage()
            .persistent()
            .get(&DataKey::Milestone(0u32))
            .unwrap();
        milestone.amount = -1;
        env.storage()
            .persistent()
            .set(&DataKey::Milestone(0u32), &milestone);
    });

    let result = client.try_fund(&client_addr);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(token.balance(&client_addr), 1_000);
    assert_eq!(token.balance(&contract_id), 0);
}

#[test]
fn test_fund_rejects_empty_milestone_set() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    // An empty milestone list is now rejected by `initialize` itself (#11),
    // so a job with zero milestones can never be persisted for `fund` to
    // reject later.
    let amounts = soroban_sdk::Vec::new(&env);
    let result = client.try_initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn test_mark_delivered_before_funded_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_mark_delivered(&freelancer_addr, &0u32);
    assert!(result.is_err());
}

#[test]
fn test_admin_add_token() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token1 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token2 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token1,
        &604800,
        &amounts,
    );

    assert!(client.is_token_whitelisted(&token1));
    assert!(!client.is_token_whitelisted(&token2));

    client.add_whitelisted_token(&admin_addr, &token2);
    assert!(client.is_token_whitelisted(&token2));

    let whitelist = client.get_whitelisted_tokens();
    assert_eq!(whitelist.len(), 2);
}

#[test]
fn test_non_admin_add_token_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let bad_actor = Address::generate(&env);

    let token1 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token2 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token1,
        &604800,
        &amounts,
    );

    let result = client.try_add_whitelisted_token(&bad_actor, &token2);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_admin_remove_token() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token1 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token2 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token1,
        &604800,
        &amounts,
    );
    client.add_whitelisted_token(&admin_addr, &token2);

    assert!(client.is_token_whitelisted(&token2));

    client.remove_whitelisted_token(&admin_addr, &token2);
    assert!(!client.is_token_whitelisted(&token2));
}

#[test]
fn test_non_admin_remove_token_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);
    let bad_actor = Address::generate(&env);

    let token1 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token2 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token1,
        &604800,
        &amounts,
    );
    client.add_whitelisted_token(&admin_addr, &token2);

    let result = client.try_remove_whitelisted_token(&bad_actor, &token2);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_add_existing_token_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token1 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token1,
        &604800,
        &amounts,
    );

    let result = client.try_add_whitelisted_token(&admin_addr, &token1);
    assert!(result.is_err());
}

#[test]
fn test_remove_nonexistent_token_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token1 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token2 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token1,
        &604800,
        &amounts,
    );

    let result = client.try_remove_whitelisted_token(&admin_addr, &token2);
    assert!(result.is_err());
}

#[test]
fn test_partial_release_remaining_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_partial(&client_addr, &0u32, &4_000_i128);

    assert_eq!(token.balance(&freelancer_addr), 4_000);
    assert_eq!(token.balance(&contract_id), 6_000);

    let job = client.get_job();
    let milestone = job.milestones.get(0).unwrap();
    assert_eq!(milestone.released_amount, 4_000);
    assert_eq!(milestone.status, MilestoneStatus::PartiallyReleased);
}

#[test]
fn test_multiple_partial_releases_sum_full() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token = token::Client::new(&env, &token_contract_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_partial(&client_addr, &0u32, &3_000_i128);
    client.approve_partial(&client_addr, &0u32, &3_000_i128);
    client.approve_partial(&client_addr, &0u32, &4_000_i128);

    assert_eq!(token.balance(&freelancer_addr), 10_000);
    assert_eq!(token.balance(&contract_id), 0);

    let job = client.get_job();
    let milestone = job.milestones.get(0).unwrap();
    assert_eq!(milestone.released_amount, 10_000);
    assert_eq!(milestone.status, MilestoneStatus::Released);
}

#[test]
fn test_over_release_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let result = client.try_approve_partial(&client_addr, &0u32, &11_000_i128);
    assert!(result.is_err());
}

#[test]
fn test_negative_or_zero_release_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let result1 = client.try_approve_partial(&client_addr, &0u32, &0_i128);
    assert!(result1.is_err());

    let result2 = client.try_approve_partial(&client_addr, &0u32, &-1000_i128);
    assert!(result2.is_err());
}

#[test]
fn test_approve_partial_large_amounts_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &i128::MAX);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, i128::MAX];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_partial(&client_addr, &0u32, &1_i128);

    // Try to approve an amount that would overflow released_amount
    let result = client.try_approve_partial(&client_addr, &0u32, &i128::MAX);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn test_release_on_wrong_status_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    client.fund(&client_addr);

    let result = client.try_approve_partial(&client_addr, &0u32, &4000_i128);
    assert!(result.is_err());
}

#[test]
fn test_approve_partial_wrong_status_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);

    // Try to approve partial on Pending status
    let result = client.try_approve_partial(&client_addr, &0u32, &4000_i128);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));

    // Mark delivered and approve fully
    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_milestone(&client_addr, &0u32);

    // Try to approve partial on Released status
    let result = client.try_approve_partial(&client_addr, &0u32, &1000_i128);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

#[test]
fn test_approve_partial_invalid_milestone_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    // Try to approve partial on non-existent milestone
    let result = client.try_approve_partial(&client_addr, &1u32, &4000_i128);
    assert_eq!(result, Err(Ok(Error::InvalidMilestone)));
}

#[test]
fn test_approve_partial_before_funded_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_approve_partial(&client_addr, &0u32, &4000_i128);
    assert_eq!(result, Err(Ok(Error::NotFunded)));
}

#[test]
fn test_approve_partial_unauthorized_partial_release_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let result = client.try_approve_partial(&freelancer_addr, &0u32, &4000_i128);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_approve_partial_arbiter_is_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let result = client.try_approve_partial(&arbiter_addr, &0u32, &4000_i128);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_approve_partial_zero_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let result = client.try_approve_partial(&client_addr, &0u32, &0_i128);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn test_approve_partial_negative_amount_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);

    let result = client.try_approve_partial(&client_addr, &0u32, &-1_i128);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn test_whitelist_state_transitions() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token1 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token2 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token3 = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token1,
        &604800,
        &amounts,
    );

    assert!(client.is_token_whitelisted(&token1));
    assert!(!client.is_token_whitelisted(&token2));

    client.add_whitelisted_token(&admin_addr, &token2);
    assert!(client.is_token_whitelisted(&token2));

    let whitelist = client.get_whitelisted_tokens();
    assert_eq!(whitelist.len(), 2);

    client.remove_whitelisted_token(&admin_addr, &token2);
    assert!(!client.is_token_whitelisted(&token2));

    client.add_whitelisted_token(&admin_addr, &token3);
    assert!(client.is_token_whitelisted(&token3));
}

#[test]
fn test_approve_partial_on_disputed_milestone_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);
    client.raise_dispute(&client_addr, &0u32);

    let result = client.try_approve_partial(&client_addr, &0u32, &4000_i128);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

#[test]
fn test_approve_partial_on_refunded_milestone_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let admin_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();
    let token_admin = token::StellarAssetClient::new(&env, &token_contract_id);
    token_admin.mint(&client_addr, &10_000);

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 10_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );
    client.fund(&client_addr);
    client.mark_delivered(&freelancer_addr, &0u32);
    client.raise_dispute(&client_addr, &0u32);
    client.resolve_dispute(&arbiter_addr, &0u32, &false);

    let result = client.try_approve_partial(&client_addr, &0u32, &4000_i128);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

// ═══════════════════════════════════════════════════════════════════════════════
// TASK 1 TESTS: multisig approval emergency admin privilege endpoints
// ═══════════════════════════════════════════════════════════════════════════════

/// Verify that only the stored admin can invoke multisig_lock.
#[test]
fn test_multisig_lock_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, _, admin_addr, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    let attacker = Address::generate(&env);
    let result = client.try_multisig_lock(&attacker);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));

    // Admin should succeed
    client.multisig_lock(&admin_addr);
    assert!(client.is_multisig_locked());
}

/// Verify multisig_lock sets the lock flag and is_multisig_locked reads it.
#[test]
fn test_multisig_lock_state_transitions() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, _, admin_addr, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    assert!(!client.is_multisig_locked());
    client.multisig_lock(&admin_addr);
    assert!(client.is_multisig_locked());
}

/// Verify multisig_admin_override_release requires verified admin auth.
#[test]
fn test_multisig_admin_override_release_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, freelancer_addr, _, admin_addr, token_id, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    let attacker = Address::generate(&env);
    let result = client.try_multisig_admin_override_release(&attacker, &0u32);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));

    // Admin override should succeed and release funds to freelancer
    let token = token::Client::new(&env, &token_id);
    let freelancer_before = token.balance(&freelancer_addr);
    client.multisig_admin_override_release(&admin_addr, &0u32);
    assert_eq!(
        token.balance(&freelancer_addr),
        freelancer_before + 1_000
    );
    // Multisig lock should be cleared
    assert!(!client.is_multisig_locked());
}

/// Verify multisig_admin_override_refund requires verified admin auth.
#[test]
fn test_multisig_admin_override_refund_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, _, _, admin_addr, token_id, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    let attacker = Address::generate(&env);
    let result = client.try_multisig_admin_override_refund(&attacker, &0u32);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));

    // Admin override should succeed and refund to client
    let token = token::Client::new(&env, &token_id);
    let client_before = token.balance(&client_addr);
    client.multisig_admin_override_refund(&admin_addr, &0u32);
    assert_eq!(token.balance(&client_addr), client_before + 1_000);
    // Multisig lock should be cleared
    assert!(!client.is_multisig_locked());
}

/// Verify multisig override release emits correct event.
#[test]
fn test_multisig_admin_override_release_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, freelancer_addr, _, admin_addr, token_id, contract_id, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.multisig_admin_override_release(&admin_addr, &0u32);

    let topic_val: Val = symbol_short!("msadmrel").into_val(&env);
    let mut found = false;
    for event in env.events().all().iter() {
        if let Some(topic) = event.1.get(0) {
            if topic.get_payload() == topic_val.get_payload() {
                found = true;
                assert_eq!(event.1.len(), 1);
                let ev = MultisigAdminOverrideReleaseEvent::from_val(&env, &event.2);
                assert_eq!(ev.admin, admin_addr);
                assert_eq!(ev.contract_id, contract_id);
                assert_eq!(ev.milestone_index, 0);
                assert_eq!(ev.freelancer, freelancer_addr);
                assert_eq!(ev.token, token_id);
                assert_eq!(ev.amount, 1_000);
            }
        }
    }
    assert!(found, "msadmrel event not emitted");
}

/// Verify multisig override refund emits correct event.
#[test]
fn test_multisig_admin_override_refund_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, _, _, admin_addr, token_id, contract_id, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.multisig_admin_override_refund(&admin_addr, &0u32);

    let topic_val: Val = symbol_short!("msadmref").into_val(&env);
    let mut found = false;
    for event in env.events().all().iter() {
        if let Some(topic) = event.1.get(0) {
            if topic.get_payload() == topic_val.get_payload() {
                found = true;
                assert_eq!(event.1.len(), 1);
                let ev = MultisigAdminOverrideRefundEvent::from_val(&env, &event.2);
                assert_eq!(ev.admin, admin_addr);
                assert_eq!(ev.contract_id, contract_id);
                assert_eq!(ev.milestone_index, 0);
                assert_eq!(ev.client, client_addr);
                assert_eq!(ev.token, token_id);
                assert_eq!(ev.amount, 1_000);
            }
        }
    }
    assert!(found, "msadmref event not emitted");
}

/// Verify multisig override release clears MultisigLocked flag.
#[test]
fn test_multisig_override_release_clears_locked_flag() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, _, admin_addr, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.multisig_lock(&admin_addr);
    assert!(client.is_multisig_locked());

    client.multisig_admin_override_release(&admin_addr, &0u32);
    assert!(!client.is_multisig_locked());
}

/// Verify multisig override refund clears MultisigLocked flag.
#[test]
fn test_multisig_override_refund_clears_locked_flag() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, _, admin_addr, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.multisig_lock(&admin_addr);
    assert!(client.is_multisig_locked());

    client.multisig_admin_override_refund(&admin_addr, &0u32);
    assert!(!client.is_multisig_locked());
}

/// Verify multisig override release on already-settled milestone fails.
#[test]
fn test_multisig_admin_override_release_on_released_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, freelancer_addr, _, admin_addr, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    // Fully release the milestone first (client must approve)
    client.mark_delivered(&freelancer_addr, &0u32);
    client.approve_milestone(&client_addr, &0u32);

    let result = client.try_multisig_admin_override_release(&admin_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

/// Verify multisig override on unfunded escrow fails.
#[test]
fn test_multisig_admin_override_release_not_funded_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin_addr = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);

    let token_contract_id = env
        .register_stellar_asset_contract_v2(admin_addr.clone())
        .address();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let amounts = vec![&env, 1_000_i128];
    client.initialize(
        &admin_addr,
        &client_addr,
        &freelancer_addr,
        &arbiter_addr,
        &token_contract_id,
        &604800,
        &amounts,
    );

    let result = client.try_multisig_admin_override_release(&admin_addr, &0u32);
    assert_eq!(result, Err(Ok(Error::NotFunded)));
}

// ═══════════════════════════════════════════════════════════════════════════════
// TASK 2 TESTS: require_dispute_party auth for raise_dispute
// ═══════════════════════════════════════════════════════════════════════════════

/// Verify raise_dispute with bad actor returns Unauthorized (not another error).
#[test]
fn test_raise_dispute_bad_actor_returns_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, freelancer_addr, _, _, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.mark_delivered(&freelancer_addr, &0u32);

    let bad_actor = Address::generate(&env);
    let result = client.try_raise_dispute(&bad_actor, &0u32);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

/// Verify raise_dispute with freelancer succeeds (freelancer is an authorized party).
#[test]
fn test_raise_dispute_by_freelancer_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, freelancer_addr, _, _, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.mark_delivered(&freelancer_addr, &0u32);
    client.raise_dispute(&freelancer_addr, &0u32);

    let job = client.get_job();
    assert_eq!(
        job.milestones.get(0).unwrap().status,
        MilestoneStatus::Disputed
    );
}

/// Verify raise_dispute by client succeeds (client is an authorized party).
#[test]
fn test_raise_dispute_by_client_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, _, _, _, _, _, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.raise_dispute(&client_addr, &0u32);

    let job = client.get_job();
    assert_eq!(
        job.milestones.get(0).unwrap().status,
        MilestoneStatus::Disputed
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TASK 3 TESTS: Temporary storage DisputeFlag optimization
// ═══════════════════════════════════════════════════════════════════════════════

/// Verify that raise_dispute writes the DisputeFlag to temporary storage.
#[test]
fn test_raise_dispute_writes_dispute_flag() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, _, _, _, _, contract_id, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    client.raise_dispute(&client_addr, &0u32);

    // Read the temporary storage flag from within the contract context
    let flag_set = env.as_contract(&contract_id, || {
        env.storage()
            .temporary()
            .get::<_, bool>(&DataKey::DisputeFlag(0u32))
            .unwrap_or(false)
    });
    assert!(flag_set, "DisputeFlag should be set in temporary storage");
}

/// Verify that DisputeFlag is NOT set before raise_dispute is called.
#[test]
fn test_dispute_flag_not_set_before_raise_dispute() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, _, _, _, contract_id, _client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128]);

    let flag_set = env.as_contract(&contract_id, || {
        env.storage()
            .temporary()
            .get::<_, bool>(&DataKey::DisputeFlag(0u32))
            .unwrap_or(false)
    });
    assert!(!flag_set, "DisputeFlag should NOT be set before raise_dispute");
}

/// Verify that only the disputed milestone's flag is set, not other indices.
#[test]
fn test_dispute_flag_only_sets_targeted_milestone() {
    let env = Env::default();
    env.mock_all_auths();

    let (client_addr, _, _, _, _, contract_id, client) =
        setup_funded_escrow(&env, vec![&env, 1_000_i128, 1_000_i128]);

    client.raise_dispute(&client_addr, &0u32);

    let flag_0 = env.as_contract(&contract_id, || {
        env.storage()
            .temporary()
            .get::<_, bool>(&DataKey::DisputeFlag(0u32))
            .unwrap_or(false)
    });
    let flag_1 = env.as_contract(&contract_id, || {
        env.storage()
            .temporary()
            .get::<_, bool>(&DataKey::DisputeFlag(1u32))
            .unwrap_or(false)
    });

    assert!(flag_0, "DisputeFlag(0) should be set");
    assert!(!flag_1, "DisputeFlag(1) should NOT be set");
}

// ═══════════════════════════════════════════════════════════════════════════════
// TASK 4 TESTS: multisig_split_refund distribution pathways
// ═══════════════════════════════════════════════════════════════════════════════

/// Verify multisig_split_refund with 50/50 split calculates correctly.
#[test]
fn test_multisig_split_refund_even_split() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let allocation = client.multisig_split_refund(&1_000_i128, &5_000_u32, &5_000_u32);
    assert_eq!(allocation.client_refund, 500);
    assert_eq!(allocation.freelancer_payout, 500);
    assert_eq!(allocation.client_refund_bps, 5_000);
    assert_eq!(allocation.freelancer_payout_bps, 5_000);
    assert_eq!(allocation.client_refund + allocation.freelancer_payout, 1_000);
}

/// Verify multisig_split_refund with 70/30 split calculates correctly.
#[test]
fn test_multisig_split_refund_uneven_split() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let allocation = client.multisig_split_refund(&1_000_i128, &7_000_u32, &3_000_u32);
    assert_eq!(allocation.client_refund, 700);
    assert_eq!(allocation.freelancer_payout, 300);
    assert_eq!(allocation.client_refund_bps, 7_000);
    assert_eq!(allocation.freelancer_payout_bps, 3_000);
    assert_eq!(allocation.client_refund + allocation.freelancer_payout, 1_000);
}

/// Verify multisig_split_refund with 100% client refund.
#[test]
fn test_multisig_split_refund_full_client_refund() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let allocation = client.multisig_split_refund(&1_000_i128, &10_000_u32, &0_u32);
    assert_eq!(allocation.client_refund, 1_000);
    assert_eq!(allocation.freelancer_payout, 0);
}

/// Verify multisig_split_refund with 100% freelancer payout.
#[test]
fn test_multisig_split_refund_full_freelancer_payout() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let allocation = client.multisig_split_refund(&1_000_i128, &0_u32, &10_000_u32);
    assert_eq!(allocation.client_refund, 0);
    assert_eq!(allocation.freelancer_payout, 1_000);
}

/// Verify multisig_split_refund rejects ratios that don't sum to BPS_SCALE.
#[test]
fn test_multisig_split_refund_invalid_ratio_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    // Total is 8000, not 10000
    let result = client.try_multisig_split_refund(&1_000_i128, &5_000_u32, &3_000_u32);
    assert_eq!(result, Err(Ok(Error::InvalidRatio)));
}

/// Verify multisig_split_refund rejects zero total amount.
#[test]
fn test_multisig_split_refund_zero_total_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let result = client.try_multisig_split_refund(&0_i128, &5_000_u32, &5_000_u32);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

/// Verify multisig_split_refund preserves total with rounding (odd amounts).
#[test]
fn test_multisig_split_refund_odd_amount_rounding() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    // 101 split 50/50 should produce 51/50 (rounding up for client)
    let allocation = client.multisig_split_refund(&101_i128, &5_000_u32, &5_000_u32);
    assert_eq!(allocation.client_refund + allocation.freelancer_payout, 101);
}

/// Verify multisig_split_refund with edge ratio 1/9999 preserves total.
#[test]
fn test_multisig_split_refund_extreme_split_preserves_total() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let allocation = client.multisig_split_refund(&10_000_i128, &1_u32, &9_999_u32);
    assert_eq!(allocation.client_refund + allocation.freelancer_payout, 10_000);
    assert_eq!(allocation.client_refund_bps, 1);
    assert_eq!(allocation.freelancer_payout_bps, 9_999);
}

/// Verify multisig_split_refund emits the correct event.
#[test]
fn test_multisig_split_refund_emits_event() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    client.multisig_split_refund(&1_000_i128, &7_000_u32, &3_000_u32);

    let topic_val: Val = symbol_short!("splitref").into_val(&env);
    let mut found = false;
    for event in env.events().all().iter() {
        if let Some(topic) = event.1.get(0) {
            if topic.get_payload() == topic_val.get_payload() {
                found = true;
                assert_eq!(event.1.len(), 1);
                let ev = SplitRefundCalculatedEvent::from_val(&env, &event.2);
                assert_eq!(ev.client_refund, 700);
                assert_eq!(ev.freelancer_payout, 300);
                assert_eq!(ev.client_refund_bps, 7_000);
                assert_eq!(ev.freelancer_payout_bps, 3_000);
            }
        }
    }
    assert!(found, "splitref event not emitted");
}

#[test]
fn test_multisig_transfer_admin_ratio_split_preserves_total() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let ratios = vec![&env, 1_i128, 1_i128, 1_i128];
    let allocations = client.multisig_transfer_admin(&100_i128, &ratios);
    assert_eq!(allocations.len(), 3);
    assert_eq!(allocations.get(0).unwrap(), 34);
    assert_eq!(allocations.get(1).unwrap(), 33);
    assert_eq!(allocations.get(2).unwrap(), 33);

    let total = allocations.iter().fold(0_i128, |acc, v| acc + v);
    assert_eq!(total, 100);
}

#[test]
fn test_multisig_transfer_admin_invalid_ratio_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(MilestoneEscrow, ());
    let client = MilestoneEscrowClient::new(&env, &contract_id);

    let ratios = vec![&env, 0_i128, 0_i128];
    let result = client.try_multisig_transfer_admin(&100_i128, &ratios);
    assert_eq!(result, Err(Ok(Error::InvalidRatio)));
}
