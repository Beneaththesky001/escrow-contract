#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    AlreadyFunded = 3,
    NotFunded = 4,
    Unauthorized = 5,
    InvalidMilestone = 6,
    InvalidStatus = 7,
    TokenNotWhitelisted = 8,
    TokenAlreadyWhitelisted = 9,
    InvalidAmount = 10,
    DeadlineNotPassed = 11,
    InvalidAddress = 12,
    Paused = 13,
    InvalidRatio = 14,
}

const BPS_SCALE: u32 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MilestoneStatus {
    Pending,
    Delivered,
    PartiallyReleased,
    Released,
    Disputed,
    Refunded,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Milestone {
    pub amount: i128,
    pub released_amount: i128,
    pub status: MilestoneStatus,
    pub delivered_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Job {
    pub client: Address,
    pub freelancer: Address,
    pub arbiter: Address,
    pub token: Address,
    pub milestones: Vec<Milestone>,
    pub funded: bool,
    pub auto_release_seconds: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
struct JobMeta {
    client: Address,
    freelancer: Address,
    arbiter: Address,
    token: Address,
    funded: bool,
    auto_release_seconds: u64,
    milestone_count: u32,
    total_amount: i128,
}

#[contracttype]
pub enum DataKey {
    Job,
    Milestone(u32),
    Admin,
    WhitelistedTokens,
    EmergencyPaused,
    PlatformFeeAllocation,
}

#[contracttype]
pub struct InitializedEvent {
    pub client: Address,
    pub freelancer: Address,
    pub arbiter: Address,
    pub token: Address,
    pub milestone_amounts: Vec<i128>,
}

#[contracttype]
pub struct FundedEvent {
    pub total_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveredEvent {
    pub contract_id: Address,
    pub milestone_index: u32,
    pub freelancer: Address,
    pub client: Address,
    pub delivered_at: u64,
    pub status: MilestoneStatus,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovedEvent {
    pub contract_id: Address,
    pub milestone_index: u32,
    pub client: Address,
    pub freelancer: Address,
    pub token: Address,
    pub amount: i128,
    pub released_amount: i128,
    pub remaining: i128,
    pub status: MilestoneStatus,
}

#[contracttype]
pub struct DisputeRaisedEvent {
    pub milestone_index: u32,
}

#[contracttype]
pub struct DisputeResolvedEvent {
    pub milestone_index: u32,
    pub released_to_freelancer: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformFeeAllocation {
    pub client_bps: u32,
    pub freelancer_bps: u32,
    pub treasury_bps: u32,
    pub locked: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RatioSplit {
    pub first: i128,
    pub second: i128,
}

#[contract]
pub struct MilestoneEscrow;

#[contractimpl]
impl MilestoneEscrow {
    fn load_admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin = Self::load_admin(env)?;
        if stored_admin != *admin {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn ensure_not_paused(env: &Env) -> Result<(), Error> {
        let paused = env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::EmergencyPaused)
            .unwrap_or(false);
        if paused {
            return Err(Error::Paused);
        }
        Ok(())
    }

    fn validate_fee_allocation(
        client_bps: u32,
        freelancer_bps: u32,
        treasury_bps: u32,
    ) -> Result<(), Error> {
        let total = client_bps
            .checked_add(freelancer_bps)
            .and_then(|v| v.checked_add(treasury_bps))
            .ok_or(Error::InvalidRatio)?;
        if total != BPS_SCALE {
            return Err(Error::InvalidRatio);
        }
        Ok(())
    }

    fn split_round_nearest(
        total: i128,
        numerator: i128,
        denominator: i128,
    ) -> Result<RatioSplit, Error> {
        if total < 0 || numerator < 0 || denominator <= 0 || numerator > denominator {
            return Err(Error::InvalidRatio);
        }

        let scaled = total.checked_mul(numerator).ok_or(Error::InvalidAmount)?;
        let half = denominator / 2;
        let rounded = scaled.checked_add(half).ok_or(Error::InvalidAmount)? / denominator;

        if rounded > total {
            return Err(Error::InvalidAmount);
        }

        Ok(RatioSplit {
            first: rounded,
            second: total.checked_sub(rounded).ok_or(Error::InvalidAmount)?,
        })
    }

    fn load_job_meta(env: &Env) -> Result<JobMeta, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Job)
            .ok_or(Error::NotInitialized)
    }

    fn store_job_meta(env: &Env, meta: &JobMeta) {
        env.storage().instance().set(&DataKey::Job, meta);
    }

    fn load_milestone(env: &Env, index: u32) -> Result<Milestone, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Milestone(index))
            .ok_or(Error::InvalidMilestone)
    }

    fn store_milestone(env: &Env, index: u32, milestone: &Milestone) {
        env.storage()
            .persistent()
            .set(&DataKey::Milestone(index), milestone);
    }

    fn checked_add_amount(total: i128, amount: i128) -> Result<i128, Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        total.checked_add(amount).ok_or(Error::InvalidAmount)
    }

    fn checked_job_total(env: &Env, meta: &JobMeta) -> Result<i128, Error> {
        let mut total_amount: i128 = 0;

        for index in 0..meta.milestone_count {
            let milestone = Self::load_milestone(env, index)?;
            total_amount = Self::checked_add_amount(total_amount, milestone.amount)?;
        }

        if total_amount != meta.total_amount {
            return Err(Error::InvalidAmount);
        }

        Ok(total_amount)
    }

    fn validate_fund_client(env: &Env, client: &Address) -> Result<(), Error> {
        if client == &env.current_contract_address() {
            return Err(Error::InvalidAddress);
        }

        Ok(())
    }

    fn assemble_job(env: &Env, meta: &JobMeta) -> Result<Job, Error> {
        let mut milestones = Vec::new(env);
        for i in 0..meta.milestone_count {
            milestones.push_back(Self::load_milestone(env, i)?);
        }
        Ok(Job {
            client: meta.client.clone(),
            freelancer: meta.freelancer.clone(),
            arbiter: meta.arbiter.clone(),
            token: meta.token.clone(),
            milestones,
            funded: meta.funded,
            auto_release_seconds: meta.auto_release_seconds,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        env: Env,
        admin: Address,
        client: Address,
        freelancer: Address,
        arbiter: Address,
        token: Address,
        auto_release_seconds: u64,
        milestone_amounts: Vec<i128>,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Job) {
            return Err(Error::AlreadyInitialized);
        }

        let milestone_count = milestone_amounts.len();
        let mut total_amount: i128 = 0;
        for amount in milestone_amounts.iter() {
            total_amount = Self::checked_add_amount(total_amount, amount)?;
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::EmergencyPaused, &false);
        env.storage().instance().set(
            &DataKey::PlatformFeeAllocation,
            &PlatformFeeAllocation {
                client_bps: 0,
                freelancer_bps: BPS_SCALE,
                treasury_bps: 0,
                locked: false,
            },
        );

        let mut whitelist: Vec<Address> = Vec::new(&env);
        whitelist.push_back(token.clone());
        env.storage()
            .instance()
            .set(&DataKey::WhitelistedTokens, &whitelist);

        for (index, amount) in milestone_amounts.iter().enumerate() {
            Self::store_milestone(
                &env,
                index as u32,
                &Milestone {
                    amount,
                    released_amount: 0,
                    status: MilestoneStatus::Pending,
                    delivered_at: 0,
                },
            );
        }

        let meta = JobMeta {
            client,
            freelancer,
            arbiter,
            token,
            funded: false,
            auto_release_seconds,
            milestone_count,
            total_amount,
        };

        Self::store_job_meta(&env, &meta);
        Ok(())
    }

    pub fn transfer_admin(
        env: Env,
        current_admin: Address,
        new_admin: Address,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &current_admin)?;

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        Ok(())
    }

    pub fn add_whitelisted_token(env: Env, admin: Address, token: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;

        let mut whitelist: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WhitelistedTokens)
            .ok_or(Error::NotInitialized)?;

        if whitelist.contains(&token) {
            return Err(Error::TokenAlreadyWhitelisted);
        }

        whitelist.push_back(token);
        env.storage()
            .instance()
            .set(&DataKey::WhitelistedTokens, &whitelist);
        Ok(())
    }

    pub fn remove_whitelisted_token(env: Env, admin: Address, token: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;

        let mut whitelist: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WhitelistedTokens)
            .ok_or(Error::NotInitialized)?;

        if let Some(index) = whitelist.iter().position(|t| t == token) {
            whitelist.remove(index as u32);
            env.storage()
                .instance()
                .set(&DataKey::WhitelistedTokens, &whitelist);
            Ok(())
        } else {
            Err(Error::TokenNotWhitelisted)
        }
    }

    pub fn is_token_whitelisted(env: Env, token: Address) -> bool {
        if let Some(whitelist) = env
            .storage()
            .instance()
            .get::<_, Vec<Address>>(&DataKey::WhitelistedTokens)
        {
            whitelist.contains(&token)
        } else {
            false
        }
    }

    pub fn get_whitelisted_tokens(env: Env) -> Result<Vec<Address>, Error> {
        env.storage()
            .instance()
            .get(&DataKey::WhitelistedTokens)
            .ok_or(Error::NotInitialized)
    }

    pub fn fund(env: Env, client: Address) -> Result<(), Error> {
        Self::ensure_not_paused(&env)?;
        Self::validate_fund_client(&env, &client)?;
        client.require_auth();
        let mut meta = Self::load_job_meta(&env)?;

        if meta.funded {
            return Err(Error::AlreadyFunded);
        }
        if meta.client != client {
            return Err(Error::Unauthorized);
        }

        let total_amount = Self::checked_job_total(&env, &meta)?;
        let token_client = token::Client::new(&env, &meta.token);
        token_client.transfer(&client, &env.current_contract_address(), &total_amount);

        meta.funded = true;
        Self::store_job_meta(&env, &meta);
        Ok(())
    }

    pub fn mark_delivered(
        env: Env,
        freelancer: Address,
        milestone_index: u32,
    ) -> Result<(), Error> {
        Self::ensure_not_paused(&env)?;
        // Check for zero addresses (both account and contract types)
        let zero_account = Address::from_str(
            &env,
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
        );
        let zero_contract = Address::from_str(
            &env,
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4",
        );

        if freelancer == zero_account || freelancer == zero_contract {
            return Err(Error::InvalidAddress);
        }
        freelancer.require_auth();

        let meta = Self::load_job_meta(&env)?;

        if meta.freelancer != freelancer {
            return Err(Error::Unauthorized);
        }
        if !meta.funded {
            return Err(Error::NotFunded);
        }
        if milestone_index >= meta.milestone_count {
            return Err(Error::InvalidMilestone);
        }

        let mut milestone = Self::load_milestone(&env, milestone_index)?;

        if milestone.status != MilestoneStatus::Pending {
            return Err(Error::InvalidStatus);
        }

        let delivered_at = env.ledger().timestamp();
        milestone.status = MilestoneStatus::Delivered;
        milestone.delivered_at = delivered_at;
        Self::store_milestone(&env, milestone_index, &milestone);

        env.events().publish(
            (symbol_short!("deliver"),),
            DeliveredEvent {
                contract_id: env.current_contract_address(),
                milestone_index,
                freelancer: meta.freelancer,
                client: meta.client,
                delivered_at,
                status: MilestoneStatus::Delivered,
                amount: milestone.amount,
            },
        );

        Ok(())
    }

    pub fn claim_auto_release(
    env: Env,
    freelancer: Address,
    milestone_index: u32,
) -> Result<(), Error> {
        Self::ensure_not_paused(&env)?;
    freelancer.require_auth();
    let meta = Self::load_job_meta(&env)?;

    if meta.freelancer != freelancer {
        return Err(Error::Unauthorized);
    }

    // 1. Validate index boundary
    if milestone_index >= meta.milestone_count {
        return Err(Error::InvalidMilestone);
    }

    let mut milestone = Self::load_milestone(&env, milestone_index)?;

    if milestone.status != MilestoneStatus::Delivered {
        return Err(Error::InvalidStatus);
    }

    // 2. Validate auto_release_seconds is non-zero
    if meta.auto_release_seconds == 0 {
        return Err(Error::InvalidAmount);
    }

    let deadline = milestone.delivered_at + meta.auto_release_seconds;
    let current = env.ledger().timestamp();
    if current < deadline {
        return Err(Error::DeadlineNotPassed);
    }

    // 3. Validate there is a positive remaining amount to release
    let remaining = milestone.amount - milestone.released_amount;
    if remaining <= 0 {
        return Err(Error::InvalidAmount);
    }

    let token_client = token::Client::new(&env, &meta.token);
    token_client.transfer(
        &env.current_contract_address(),
        &meta.freelancer,
        &remaining,
    );

    milestone.released_amount = milestone.amount;
    milestone.status = MilestoneStatus::Released;
    Self::store_milestone(&env, milestone_index, &milestone);
    Ok(())
}

    pub fn time_until_auto_release(env: Env, milestone_index: u32) -> i64 {
        let meta = Self::load_job_meta(&env).unwrap();
        let milestone = Self::load_milestone(&env, milestone_index).unwrap();
        let deadline = milestone.delivered_at + meta.auto_release_seconds;
        let current = env.ledger().timestamp();
        (deadline as i64) - (current as i64)
    }

    pub fn approve_partial(
        env: Env,
        client: Address,
        milestone_index: u32,
        amount: i128,
    ) -> Result<(), Error> {
        Self::ensure_not_paused(&env)?;
        client.require_auth();
        let meta = Self::load_job_meta(&env)?;

        if meta.client != client {
            return Err(Error::Unauthorized);
        }

        if milestone_index >= meta.milestone_count {
            return Err(Error::InvalidMilestone);
        }

        let milestone = Self::load_milestone(&env, milestone_index)?;

        if milestone.status != MilestoneStatus::Delivered
            && milestone.status != MilestoneStatus::PartiallyReleased
        {
            return Err(Error::InvalidStatus);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let remaining = milestone.amount.checked_sub(milestone.released_amount).ok_or(Error::InvalidAmount)?;
        if amount > remaining {
            return Err(Error::InvalidAmount);
        }

        let token_client = token::Client::new(&env, &meta.token);
        token_client.transfer(&env.current_contract_address(), &meta.freelancer, &amount);

        let mut updated_milestone = milestone;
        updated_milestone.released_amount = updated_milestone.released_amount.checked_add(amount).ok_or(Error::InvalidAmount)?;

        if updated_milestone.released_amount == updated_milestone.amount {
            updated_milestone.status = MilestoneStatus::Released;
        } else {
            updated_milestone.status = MilestoneStatus::PartiallyReleased;
        }

        Self::store_milestone(&env, milestone_index, &updated_milestone);

        let event_remaining = updated_milestone.amount.checked_sub(updated_milestone.released_amount).ok_or(Error::InvalidAmount)?;
        env.events().publish(
            (symbol_short!("approve"),),
            ApprovedEvent {
                contract_id: env.current_contract_address(),
                milestone_index,
                client: meta.client,
                freelancer: meta.freelancer,
                token: meta.token,
                amount,
                released_amount: updated_milestone.released_amount,
                remaining: event_remaining,
                status: updated_milestone.status.clone(),
            },
        );

        Ok(())
    }

    pub fn approve_milestone(env: Env, client: Address, milestone_index: u32) -> Result<(), Error> {
        Self::ensure_not_paused(&env)?;
        client.require_auth();
        let meta = Self::load_job_meta(&env)?;

        if meta.client != client {
            return Err(Error::Unauthorized);
        }

        if milestone_index >= meta.milestone_count {
            return Err(Error::InvalidMilestone);
        }

        let mut milestone = Self::load_milestone(&env, milestone_index)?;

        if milestone.amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if milestone.status != MilestoneStatus::Delivered
            && milestone.status != MilestoneStatus::PartiallyReleased
        {
            return Err(Error::InvalidStatus);
        }

        let remaining = milestone.amount.checked_sub(milestone.released_amount).ok_or(Error::InvalidAmount)?;
        if remaining <= 0 {
            return Err(Error::InvalidAmount);
        }

        let token_client = token::Client::new(&env, &meta.token);
        token_client.transfer(
            &env.current_contract_address(),
            &meta.freelancer,
            &remaining,
        );
        milestone.released_amount = milestone.amount;

        milestone.status = MilestoneStatus::Released;
        Self::store_milestone(&env, milestone_index, &milestone);

        env.events().publish(
            (symbol_short!("approve"),),
            ApprovedEvent {
                contract_id: env.current_contract_address(),
                milestone_index,
                client: meta.client,
                freelancer: meta.freelancer,
                token: meta.token,
                amount: remaining,
                released_amount: milestone.released_amount,
                remaining: milestone.amount - milestone.released_amount,
                status: milestone.status.clone(),
            },
        );

        Ok(())
    }

    pub fn raise_dispute(env: Env, caller: Address, milestone_index: u32) -> Result<(), Error> {
        Self::ensure_not_paused(&env)?;
        caller.require_auth();
        let meta = Self::load_job_meta(&env)?;

        if meta.client != caller && meta.freelancer != caller {
            return Err(Error::Unauthorized);
        }

        let mut milestone = Self::load_milestone(&env, milestone_index)?;

        if milestone.status != MilestoneStatus::Pending
            && milestone.status != MilestoneStatus::Delivered
            && milestone.status != MilestoneStatus::PartiallyReleased
        {
            return Err(Error::InvalidStatus);
        }

        milestone.status = MilestoneStatus::Disputed;
        Self::store_milestone(&env, milestone_index, &milestone);
        Ok(())
    }

    pub fn resolve_dispute(
        env: Env,
        arbiter: Address,
        milestone_index: u32,
        release_to_freelancer: bool,
    ) -> Result<(), Error> {
        Self::ensure_not_paused(&env)?;
        arbiter.require_auth();
        let meta = Self::load_job_meta(&env)?;

        if meta.arbiter != arbiter {
            return Err(Error::Unauthorized);
        }

        let mut milestone = Self::load_milestone(&env, milestone_index)?;

        if milestone.status != MilestoneStatus::Disputed {
            return Err(Error::InvalidStatus);
        }

        let remaining = milestone.amount - milestone.released_amount;
        let token_client = token::Client::new(&env, &meta.token);
        if release_to_freelancer {
            if remaining > 0 {
                token_client.transfer(
                    &env.current_contract_address(),
                    &meta.freelancer,
                    &remaining,
                );
                milestone.released_amount = milestone.amount;
            }
            milestone.status = MilestoneStatus::Released;
        } else {
            if remaining > 0 {
                token_client.transfer(&env.current_contract_address(), &meta.client, &remaining);
            }
            milestone.status = MilestoneStatus::Refunded;
        }

        Self::store_milestone(&env, milestone_index, &milestone);
        Ok(())
    }

    pub fn emergency_pause(env: Env, admin: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::EmergencyPaused, &true);
        Ok(())
    }

    pub fn emergency_unpause(env: Env, admin: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::EmergencyPaused, &false);
        Ok(())
    }

    pub fn emergency_pause_admin_override(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::EmergencyPaused, &paused);
        Ok(())
    }

    pub fn is_emergency_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::EmergencyPaused)
            .unwrap_or(false)
    }

    pub fn set_platform_fee_allocation(
        env: Env,
        admin: Address,
        client_bps: u32,
        freelancer_bps: u32,
        treasury_bps: u32,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        Self::validate_fee_allocation(client_bps, freelancer_bps, treasury_bps)?;

        let current: PlatformFeeAllocation = env
            .storage()
            .instance()
            .get(&DataKey::PlatformFeeAllocation)
            .ok_or(Error::NotInitialized)?;

        if current.locked {
            return Err(Error::InvalidStatus);
        }

        env.storage().instance().set(
            &DataKey::PlatformFeeAllocation,
            &PlatformFeeAllocation {
                client_bps,
                freelancer_bps,
                treasury_bps,
                locked: false,
            },
        );
        Ok(())
    }

    pub fn lock_platform_fee_allocation(env: Env, admin: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        let mut current: PlatformFeeAllocation = env
            .storage()
            .instance()
            .get(&DataKey::PlatformFeeAllocation)
            .ok_or(Error::NotInitialized)?;
        current.locked = true;
        env.storage()
            .instance()
            .set(&DataKey::PlatformFeeAllocation, &current);
        Ok(())
    }

    pub fn platform_fee_allocation_admin_override(
        env: Env,
        admin: Address,
        client_bps: u32,
        freelancer_bps: u32,
        treasury_bps: u32,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        Self::validate_fee_allocation(client_bps, freelancer_bps, treasury_bps)?;
        env.storage().instance().set(
            &DataKey::PlatformFeeAllocation,
            &PlatformFeeAllocation {
                client_bps,
                freelancer_bps,
                treasury_bps,
                locked: false,
            },
        );
        Ok(())
    }

    pub fn get_platform_fee_allocation(env: Env) -> Result<PlatformFeeAllocation, Error> {
        env.storage()
            .instance()
            .get(&DataKey::PlatformFeeAllocation)
            .ok_or(Error::NotInitialized)
    }

    pub fn payment_streaming_milestones(
        _env: Env,
        total_amount: i128,
        numerator: i128,
        denominator: i128,
    ) -> Result<RatioSplit, Error> {
        Self::split_round_nearest(total_amount, numerator, denominator)
    }

    pub fn multisig_transfer_admin(
        env: Env,
        total_amount: i128,
        ratios: Vec<i128>,
    ) -> Result<Vec<i128>, Error> {
        if total_amount < 0 || ratios.is_empty() {
            return Err(Error::InvalidRatio);
        }

        let mut ratio_sum: i128 = 0;
        for ratio in ratios.iter() {
            if ratio < 0 {
                return Err(Error::InvalidRatio);
            }
            ratio_sum = ratio_sum.checked_add(ratio).ok_or(Error::InvalidRatio)?;
        }

        if ratio_sum <= 0 {
            return Err(Error::InvalidRatio);
        }

        let mut allocations: Vec<i128> = Vec::new(&env);
        let mut remainders: Vec<i128> = Vec::new(&env);
        let mut allocated_total: i128 = 0;

        for ratio in ratios.iter() {
            let weighted = total_amount.checked_mul(ratio).ok_or(Error::InvalidAmount)?;
            let base = weighted / ratio_sum;
            let rem = weighted % ratio_sum;

            allocations.push_back(base);
            remainders.push_back(rem);
            allocated_total = allocated_total.checked_add(base).ok_or(Error::InvalidAmount)?;
        }

        let remaining = total_amount
            .checked_sub(allocated_total)
            .ok_or(Error::InvalidAmount)?;

        for _ in 0..remaining {
            let mut best_index: u32 = 0;
            let mut best_remainder: i128 = i128::MIN;

            for (idx, rem) in remainders.iter().enumerate() {
                if rem > best_remainder {
                    best_remainder = rem;
                    best_index = idx as u32;
                }
            }

            let current = allocations.get(best_index).ok_or(Error::InvalidAmount)?;
            allocations.set(
                best_index,
                current.checked_add(1).ok_or(Error::InvalidAmount)?,
            );
            remainders.set(best_index, i128::MIN);
        }

        Ok(allocations)
    }

    pub fn get_job(env: Env) -> Result<Job, Error> {
        let meta = Self::load_job_meta(&env)?;
        Self::assemble_job(&env, &meta)
    }
}

mod test;
