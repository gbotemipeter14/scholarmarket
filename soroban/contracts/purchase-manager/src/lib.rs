#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Bytes, BytesN, Env,
    IntoVal, Symbol, Val, Vec,
};

pub mod auth;

const BASIS_POINTS: u32 = 10_000;
const MAX_PLATFORM_FEE_BPS: u32 = 1_000;
const MAX_PAYOUT_RECIPIENTS: u32 = 5;
const ESCROW_LOCK_PERIOD_LEDGERS: u32 = 35_000;
const DISPUTE_WINDOW_LEDGERS: u32 = 30_000;

/// TTL renewal policy (#464): whenever a tracked entry's remaining TTL drops
/// below half of the network's configured maximum, extend it back out to
/// the maximum. See ../../docs/ttl-operations.md for the operational
/// rationale, renewal cadence, and alert thresholds.
const TTL_RENEWAL_DIVISOR: u32 = 2;

/// Upper bound on how many records a single maintenance call will touch, so
/// a TTL-renewal sweep can never exceed a transaction's resource limits
/// regardless of what a caller passes as `limit`. Mirrors the same
/// footprint-budget reasoning as material-registry's constant of the same
/// name — verified directly by this contract's maintenance-sweep tests.
const MAX_MAINTENANCE_BATCH: u32 = 25;

/// Volume-tier discounted fee rates (basis points).
/// Tier 1: 2.5 %, Tier 2: 1.5 %.
const TIER1_FEE_BPS: u32 = 250;
const TIER2_FEE_BPS: u32 = 150;

/// Material status from registry (replicated here to avoid circular deps)
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterialStatus {
    Active = 0,
    Paused = 1,
    Archived = 2,
}

/// Volume tier assigned to a creator that controls the platform fee rate they pay.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreatorTier {
    /// Standard rate — uses the global `platform_fee_bps` from `PlatformConfig`.
    Default = 0,
    /// High-volume tier: 2.5 % platform fee.
    Tier1 = 1,
    /// Premium-volume tier: 1.5 % platform fee.
    Tier2 = 2,
}

/// Classification of a Stellar asset accepted by the purchase manager.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetKind {
    Native = 0,
    Token = 1,
    CreatorToken = 2,
}

/// Settlement state machine for a purchase.
///
/// Each purchase reaches exactly one terminal settlement state.
/// - `Pending`:   After purchase, escrow locked, entitlement active.
/// - `Released`:  Escrow released to creator (terminal). Entitlement stays active.
/// - `Disputed`:  Buyer opened a dispute before lock period expired.
/// - `Refunded`:  Buyer refunded, entitlement revoked (terminal).
/// - `Expired`:   Timeout reached without action (terminal).
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettlementState {
    Pending = 0,
    Released = 1,
    Disputed = 2,
    Refunded = 3,
    Expired = 4,
}

/// Resolution applied by admin when closing a dispute.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisputeResolution {
    /// Not yet resolved (default/unresolved state).
    Unresolved = 0,
    /// Refund the buyer and revoke entitlement.
    RefundBuyer = 1,
    /// Release funds to the creator, keep entitlement active.
    ReleaseToCreator = 2,
}

/// Allowlist record stored for each approved payment asset.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetInfo {
    pub kind: AssetKind,
    pub enabled: bool,
}

/// Asset quote structure from registry
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetQuote {
    pub asset: Address,
    pub amount: i128,
}

/// Payout share structure from registry
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutShare {
    pub recipient: Address,
    pub share_bps: u32,
}

/// Material record structure (minimal fields needed from registry)
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterialRecord {
    pub material_id: BytesN<32>,
    pub creator: Address,
    pub paused: bool,
    pub status: MaterialStatus,
    pub quotes: Vec<AssetQuote>,
    pub payout_shares: Vec<PayoutShare>,
}

/// Platform configuration stored in PurchaseManager
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformConfig {
    pub registry: Address,
    pub treasury: Address,
    pub platform_fee_bps: u32,
    pub paused: bool,
    /// Optional price-oracle address for future cross-asset conversion support.
    pub oracle: Option<Address>,
}

/// Entitlement record for a successful purchase
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntitlementRecord {
    pub material_id: BytesN<32>,
    pub buyer: Address,
    pub active: bool,
    pub purchase_id: u64,
    pub asset: Address,
    pub amount: i128,
    pub granted_ledger: u32,
}

/// Escrow record holding creator payout funds during the cooling-off period
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowRecord {
    pub purchase_id: u64,
    pub material_id: BytesN<32>,
    pub asset: Address,
    pub total_amount: i128,
    pub platform_fee: i128,
    pub seller_net: i128,
    pub payout_shares: Vec<PayoutShare>,
    pub purchase_ledger: u32,
    pub claimed: bool,
}

/// Settlement record tracking the lifecycle state of a purchase.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementRecord {
    pub purchase_id: u64,
    pub state: SettlementState,
    pub disputed_ledger: Option<u32>,
    pub resolved_ledger: Option<u32>,
    pub refunded_amount: i128,
}

/// Dispute record with details about an opened dispute.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeRecord {
    pub purchase_id: u64,
    pub opener: Address,
    pub reason: Bytes,
    pub opened_ledger: u32,
    /// Resolution status: `Unresolved` = pending, `RefundBuyer` or `ReleaseToCreator` = resolved.
    pub resolution: DisputeResolution,
    pub resolved_ledger: Option<u32>,
}

/// Data keys for contract storage
#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// Instance storage (#464 Tier A) — shares the contract instance's own
    /// TTL, so it never needs independent renewal.
    PlatformConfig,
    PurchaseNonce,
    PendingAdmin,
    AllowedAsset(Address),
    Entitlement((BytesN<32>, Address)),
    Escrow(u64),
    Settlement(u64),
    Dispute(u64),
    PurchaseBuyer(u64),
    CreatorTier(Address),
    /// Maintenance index (#464): sequential slot -> (purchase_id,
    /// material_id, buyer), populated at purchase time so
    /// `extend_purchases_ttl` can renew both the `Escrow` and `Entitlement`
    /// halves of a purchase together without off-chain enumeration.
    PurchaseIndex(u64),
    /// Instance storage: total number of `PurchaseIndex` slots populated.
    PurchaseIndexCount,
    /// Maintenance index (#464): sequential slot -> asset address.
    AllowedAssetIndex(u64),
    /// Instance storage: total number of `AllowedAssetIndex` slots populated.
    AllowedAssetIndexCount,
    /// Maintenance index (#464): sequential slot -> creator address.
    CreatorTierIndex(u64),
    /// Instance storage: total number of `CreatorTierIndex` slots populated.
    CreatorTierIndexCount,
    /// Instance storage: total number of `auth::AuthDataKey::AdminRoleIndex`
    /// slots populated. Lives here (rather than in `auth`) purely so every
    /// maintenance counter is defined in one place; the index entries
    /// themselves stay in `auth`'s own storage namespace.
    AdminRoleIndexCount,
}

/// Contract errors
#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum PurchaseError {
    // Initialization errors
    AlreadyInitialized = 1,
    InvalidPlatformFee = 2,

    // Purchase validation errors
    ContractPaused = 10,
    MaterialNotActive = 11,
    AssetNotAllowed = 12,
    InvalidQuoteAmount = 13,
    AssetNotAcceptedForMaterial = 14,
    EntitlementAlreadyExists = 15,

    // Payout errors
    PayoutTransferFailed = 20,
    InvalidPayoutShares = 21,

    // Registry errors
    RegistryCallFailed = 30,
    MaterialNotFound = 31,

    // Admin errors
    NotAuthorized = 40,
    InvalidTreasury = 41,
    UpgradeFailed = 42,

    // Escrow errors
    EscrowLocked = 50,
    EscrowAlreadyClaimed = 51,

    // Admin transfer errors
    NoPendingAdminTransfer = 60,

    // Settlement / Dispute errors
    SettlementNotPending = 70,
    DisputeWindowExpired = 71,
    DisputeAlreadyExists = 72,
    NoActiveDispute = 73,
    DisputeNotResolved = 74,
    InvalidDisputeReason = 75,
    PurchaseAlreadySettled = 76,
    RefundNotAllowed = 77,
    InsufficientEscrowBalance = 78,
}

/// Event: purchase.completed
#[contractevent(topics = ["purchase", "completed"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurchaseCompletedEvent {
    #[topic]
    pub purchase_id: u64,
    #[topic]
    pub material_id: BytesN<32>,
    #[topic]
    pub buyer: Address,
    pub seller: Address,
    pub asset: Address,
    pub amount: i128,
    pub platform_fee: i128,
    pub seller_net_amount: i128,
    pub entitlement_active: bool,
    pub transaction_id: Bytes,
}

/// Event: payout.distributed
#[contractevent(topics = ["payout", "distributed"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutDistributedEvent {
    #[topic]
    pub purchase_id: u64,
    #[topic]
    pub material_id: BytesN<32>,
    #[topic]
    pub recipient: Address,
    pub role: Symbol,
    pub asset: Address,
    pub amount: i128,
    pub transaction_id: Bytes,
}

/// Event: admin.asset_policy_updated
#[contractevent(topics = ["admin", "asset_policy_updated"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetPolicyUpdatedEvent {
    #[topic]
    pub asset: Address,
    pub kind: AssetKind,
    pub enabled: bool,
}

/// Event: admin.platform_config_updated
#[contractevent(topics = ["admin", "platform_config_updated"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformConfigUpdatedEvent {
    pub treasury: Address,
    pub platform_fee_bps: u32,
    pub paused: bool,
}

/// Event: escrow.created
#[contractevent(topics = ["escrow", "created"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowCreatedEvent {
    #[topic]
    pub purchase_id: u64,
    #[topic]
    pub material_id: BytesN<32>,
    pub asset: Address,
    pub amount: i128,
    pub lock_until_ledger: u32,
}

/// Event: escrow.released
#[contractevent(topics = ["escrow", "released"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowReleasedEvent {
    #[topic]
    pub purchase_id: u64,
    pub material_id: BytesN<32>,
    pub asset: Address,
    pub amount: i128,
}

/// Event: dispute.opened
#[contractevent(topics = ["dispute", "opened"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeOpenedEvent {
    #[topic]
    pub purchase_id: u64,
    #[topic]
    pub material_id: BytesN<32>,
    #[topic]
    pub opener: Address,
    pub reason: Bytes,
    pub opened_ledger: u32,
}

/// Event: dispute.resolved
#[contractevent(topics = ["dispute", "resolved"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeResolvedEvent {
    #[topic]
    pub purchase_id: u64,
    #[topic]
    pub material_id: BytesN<32>,
    pub resolution: DisputeResolution,
    pub resolved_ledger: u32,
}

/// Event: purchase.refunded
#[contractevent(topics = ["purchase", "refunded"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurchaseRefundedEvent {
    #[topic]
    pub purchase_id: u64,
    #[topic]
    pub material_id: BytesN<32>,
    #[topic]
    pub buyer: Address,
    pub asset: Address,
    pub refund_amount: i128,
    pub entitlement_revoked: bool,
}

/// Event: admin.transfer_initiated
#[contractevent(topics = ["admin", "transfer_initiated"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferInitiatedEvent {
    #[topic]
    pub from: Address,
    pub pending_admin: Address,
}

/// Event: admin.transfer_accepted
#[contractevent(topics = ["admin", "transfer_accepted"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferAcceptedEvent {
    #[topic]
    pub new_admin: Address,
}

/// Event: creator.tier_updated
#[contractevent(topics = ["creator", "tier_updated"], data_format = "vec")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatorTierUpdatedEvent {
    #[topic]
    pub creator: Address,
    pub tier: CreatorTier,
}

/// The PurchaseManager contract
#[contract]
pub struct PurchaseManager;

// ============== SAC Token Interface (SEP-41) ==============

/// Wrapper around a Stellar Asset Contract (SAC) address that exposes the
/// SEP-41 token interface methods needed by the purchase flow.
pub struct SacToken<'a> {
    env: &'a Env,
    address: &'a Address,
}

impl<'a> SacToken<'a> {
    pub fn new(env: &'a Env, address: &'a Address) -> Self {
        SacToken { env, address }
    }

    /// Transfer `amount` tokens from `from` to `to`.
    pub fn transfer(&self, from: &Address, to: &Address, amount: i128) {
        let func = Symbol::new(self.env, "transfer");
        let args = Vec::from_array(
            self.env,
            [
                from.into_val(self.env),
                to.into_val(self.env),
                amount.into_val(self.env),
            ],
        );
        self.env.invoke_contract::<()>(self.address, &func, args);
    }

    /// Query the token balance of `id`.
    pub fn balance(&self, id: &Address) -> i128 {
        let func = Symbol::new(self.env, "balance");
        let args = Vec::from_array(self.env, [id.into_val(self.env)]);
        self.env.invoke_contract::<i128>(self.address, &func, args)
    }
}

// ============== Price Oracle Stub ==============

/// Stub for a future price-oracle integration (e.g. Reflector Oracle / SEP-40).
pub struct PriceOracle<'a> {
    env: &'a Env,
    address: &'a Address,
}

impl<'a> PriceOracle<'a> {
    pub fn new(env: &'a Env, address: &'a Address) -> Self {
        PriceOracle { env, address }
    }

    pub fn last_price(&self, _base: &Address, _quote: &Address) -> Option<(i128, u32)> {
        let _ = (self.env, self.address);
        None
    }
}

// ============== Registry Cross-Contract Interface ==============

/// Interface for calling MaterialRegistry contract
pub trait MaterialRegistryInterface {
    fn get_material(
        &self,
        env: &Env,
        material_id: &BytesN<32>,
    ) -> Result<MaterialRecord, PurchaseError>;
}

/// Interface implementation using cross-contract call
impl MaterialRegistryInterface for Address {
    fn get_material(
        &self,
        env: &Env,
        material_id: &BytesN<32>,
    ) -> Result<MaterialRecord, PurchaseError> {
        let func = Symbol::new(env, "get_material");
        let result: Result<MaterialRecord, PurchaseError> = env.invoke_contract(
            self,
            &func,
            Vec::from_array(env, [material_id.into_val(env)]),
        );
        result.map_err(|_| PurchaseError::RegistryCallFailed)
    }
}

#[contractimpl]
impl PurchaseManager {
    /// Initialize the PurchaseManager contract with platform configuration
    pub fn initialize(
        env: Env,
        admin: Address,
        registry: Address,
        treasury: Address,
        platform_fee_bps: u32,
    ) -> Result<(), PurchaseError> {
        admin.require_auth();

        // Check if already initialized
        if env.storage().instance().has(&DataKey::PlatformConfig) {
            return Err(PurchaseError::AlreadyInitialized);
        }

        if platform_fee_bps > MAX_PLATFORM_FEE_BPS {
            return Err(PurchaseError::InvalidPlatformFee);
        }

        if treasury == env.current_contract_address() {
            return Err(PurchaseError::InvalidTreasury);
        }

        let config = PlatformConfig {
            registry,
            treasury: treasury.clone(),
            platform_fee_bps,
            paused: false,
            oracle: None,
        };

        auth::set_admin_role(&env, &admin);
        put_platform_config(&env, &config);
        env.storage().instance().set(&DataKey::PurchaseNonce, &0u64);
        extend_instance_ttl(&env);

        PlatformConfigUpdatedEvent {
            treasury,
            platform_fee_bps,
            paused: false,
        }
        .publish(&env);

        Ok(())
    }

    /// Execute a purchase for a material
    pub fn purchase(
        env: Env,
        buyer: Address,
        material_id: BytesN<32>,
        asset: Address,
        expected_amount: i128,
        transaction_id: Bytes,
    ) -> Result<u64, PurchaseError> {
        buyer.require_auth();

        let config = get_platform_config(&env)?;

        if config.paused {
            return Err(PurchaseError::ContractPaused);
        }

        if !is_asset_allowed(&env, &asset) {
            return Err(PurchaseError::AssetNotAllowed);
        }

        if has_entitlement_internal(&env, &material_id, &buyer) {
            return Err(PurchaseError::EntitlementAlreadyExists);
        }

        let material: MaterialRecord = config
            .registry
            .get_material(&env, &material_id)
            .map_err(|_| PurchaseError::MaterialNotFound)?;

        if material.status != MaterialStatus::Active || material.paused {
            return Err(PurchaseError::MaterialNotActive);
        }

        let quote = find_quote(&material.quotes, &asset)
            .ok_or(PurchaseError::AssetNotAcceptedForMaterial)?;

        if quote.amount != expected_amount {
            return Err(PurchaseError::InvalidQuoteAmount);
        }
        if quote.amount <= 0 {
            return Err(PurchaseError::InvalidQuoteAmount);
        }

        validate_payout_shares(&material.payout_shares)?;

        let gross = quote.amount;
        let effective_fee_bps =
            get_effective_fee_bps(&env, &material.creator, config.platform_fee_bps);
        let platform_fee = (gross * effective_fee_bps as i128) / BASIS_POINTS as i128;
        let seller_net = gross - platform_fee;

        let purchase_id = get_and_increment_purchase_nonce(&env)?;
        let current_ledger = env.ledger().sequence();

        if platform_fee > 0 {
            transfer_asset(&env, &buyer, &config.treasury, &asset, platform_fee)?;

            PayoutDistributedEvent {
                purchase_id,
                material_id: material_id.clone(),
                recipient: config.treasury.clone(),
                role: Symbol::new(&env, "platform_fee"),
                asset: asset.clone(),
                amount: platform_fee,
                transaction_id: transaction_id.clone(),
            }
            .publish(&env);
        }

        if seller_net > 0 {
            transfer_asset(
                &env,
                &buyer,
                &env.current_contract_address(),
                &asset,
                seller_net,
            )?;
        }

        let escrow_record = EscrowRecord {
            purchase_id,
            material_id: material_id.clone(),
            asset: asset.clone(),
            total_amount: gross,
            platform_fee,
            seller_net,
            payout_shares: material.payout_shares.clone(),
            purchase_ledger: current_ledger,
            claimed: false,
        };
        set_escrow_record(&env, purchase_id, &escrow_record);

        EscrowCreatedEvent {
            purchase_id,
            material_id: material_id.clone(),
            asset: asset.clone(),
            amount: seller_net,
            lock_until_ledger: current_ledger + ESCROW_LOCK_PERIOD_LEDGERS,
        }
        .publish(&env);

        let entitlement = EntitlementRecord {
            material_id: material_id.clone(),
            buyer: buyer.clone(),
            active: true,
            purchase_id,
            asset: asset.clone(),
            amount: gross,
            granted_ledger: current_ledger,
        };

        set_entitlement(&env, &entitlement);
        index_purchase(&env, purchase_id, &material_id, &buyer);

        // Store purchase → buyer mapping for admin refunds
        env.storage()
            .persistent()
            .set(&DataKey::PurchaseBuyer(purchase_id), &buyer);

        // Initialize settlement record in Pending state
        let settlement = SettlementRecord {
            purchase_id,
            state: SettlementState::Pending,
            disputed_ledger: None,
            resolved_ledger: None,
            refunded_amount: 0,
        };
        set_settlement_record(&env, purchase_id, &settlement);

        PurchaseCompletedEvent {
            purchase_id,
            material_id: material_id.clone(),
            buyer: buyer.clone(),
            seller: material.creator.clone(),
            asset: asset.clone(),
            amount: gross,
            platform_fee,
            seller_net_amount: seller_net,
            entitlement_active: true,
            transaction_id,
        }
        .publish(&env);

        Ok(purchase_id)
    }

    /// Check if a buyer has an active entitlement for a material
    pub fn has_entitlement(env: Env, material_id: BytesN<32>, buyer: Address) -> bool {
        has_entitlement_internal(&env, &material_id, &buyer)
    }

    /// Get entitlement details for a buyer and material
    pub fn get_entitlement(
        env: Env,
        material_id: BytesN<32>,
        buyer: Address,
    ) -> Option<EntitlementRecord> {
        get_entitlement_internal(&env, &material_id, &buyer)
    }

    /// Get current platform configuration
    pub fn get_platform_config(env: Env) -> Option<PlatformConfig> {
        get_platform_config(&env).ok()
    }

    /// Check if an asset is globally allowed for purchases
    pub fn is_asset_allowed(env: Env, asset: Address) -> bool {
        is_asset_allowed(&env, &asset)
    }

    /// Withdraw escrowed creator payout funds after the lock period expires.
    /// Only callable by a payout recipient (e.g., the creator).
    /// Transitions settlement from Pending → Released.
    pub fn withdraw_payouts(
        env: Env,
        caller: Address,
        purchase_id: u64,
    ) -> Result<(), PurchaseError> {
        caller.require_auth();

        let mut escrow =
            get_escrow_record_internal(&env, purchase_id).ok_or(PurchaseError::MaterialNotFound)?;

        if escrow.claimed {
            return Err(PurchaseError::EscrowAlreadyClaimed);
        }

        // Verify settlement is in Pending state (not disputed, refunded, or already released)
        let mut settlement = get_settlement_record_internal(&env, purchase_id)
            .ok_or(PurchaseError::SettlementNotPending)?;
        if settlement.state != SettlementState::Pending {
            return Err(PurchaseError::SettlementNotPending);
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger < escrow.purchase_ledger + ESCROW_LOCK_PERIOD_LEDGERS {
            return Err(PurchaseError::EscrowLocked);
        }

        let caller_is_recipient = {
            let mut index = 0;
            let mut found = false;
            while index < escrow.payout_shares.len() {
                if escrow.payout_shares.get_unchecked(index).recipient == caller {
                    found = true;
                    break;
                }
                index += 1;
            }
            found
        };

        if !caller_is_recipient {
            return Err(PurchaseError::NotAuthorized);
        }

        if escrow.seller_net > 0 {
            distribute_payout_shares_from_contract(
                &env,
                purchase_id,
                &escrow.material_id,
                &escrow,
            )?;
        }

        escrow.claimed = true;
        set_escrow_record(&env, purchase_id, &escrow);

        // Transition settlement to Released
        settlement.state = SettlementState::Released;
        settlement.resolved_ledger = Some(current_ledger);
        set_settlement_record(&env, purchase_id, &settlement);

        EscrowReleasedEvent {
            purchase_id,
            material_id: escrow.material_id,
            asset: escrow.asset,
            amount: escrow.seller_net,
        }
        .publish(&env);

        Ok(())
    }

    /// Get the escrow record for a purchase
    pub fn get_escrow_record(env: Env, purchase_id: u64) -> Option<EscrowRecord> {
        get_escrow_record_internal(&env, purchase_id)
    }

    /// Check if the escrow for a purchase can be released
    pub fn is_escrow_releasable(env: Env, purchase_id: u64) -> bool {
        if let Some(escrow) = get_escrow_record_internal(&env, purchase_id) {
            if escrow.claimed {
                return false;
            }
            // Also check settlement state — must be Pending
            if let Some(settlement) = get_settlement_record_internal(&env, purchase_id) {
                if settlement.state != SettlementState::Pending {
                    return false;
                }
            }
            let current_ledger = env.ledger().sequence();
            return current_ledger >= escrow.purchase_ledger + ESCROW_LOCK_PERIOD_LEDGERS;
        }
        false
    }

    // ============== Dispute Functions ==============

    /// Open a dispute on a purchase.
    ///
    /// Only the buyer who made the purchase can open a dispute.
    /// The dispute must be opened within the dispute window (before lock period expires).
    /// Transitions settlement from Pending → Disputed.
    pub fn open_dispute(
        env: Env,
        buyer: Address,
        purchase_id: u64,
        reason: Bytes,
    ) -> Result<(), PurchaseError> {
        buyer.require_auth();

        // Validate reason is not empty
        if reason.len() == 0 {
            return Err(PurchaseError::InvalidDisputeReason);
        }

        let escrow =
            get_escrow_record_internal(&env, purchase_id).ok_or(PurchaseError::MaterialNotFound)?;

        // Verify the caller is the buyer who made the purchase
        let entitlement_key = DataKey::Entitlement((escrow.material_id.clone(), buyer.clone()));
        let entitlement: EntitlementRecord = env
            .storage()
            .persistent()
            .get(&entitlement_key)
            .ok_or(PurchaseError::NotAuthorized)?;

        if !entitlement.active {
            return Err(PurchaseError::NotAuthorized);
        }

        // Check no dispute already exists for this purchase (must be first)
        if env.storage().persistent().has(&DataKey::Dispute(purchase_id)) {
            return Err(PurchaseError::DisputeAlreadyExists);
        }

        // Verify settlement is in Pending state
        let mut settlement = get_settlement_record_internal(&env, purchase_id)
            .ok_or(PurchaseError::SettlementNotPending)?;
        if settlement.state != SettlementState::Pending {
            return Err(PurchaseError::SettlementNotPending);
        }

        // Dispute must be opened within the dispute window
        let current_ledger = env.ledger().sequence();
        if current_ledger >= escrow.purchase_ledger + DISPUTE_WINDOW_LEDGERS {
            return Err(PurchaseError::DisputeWindowExpired);
        }

        // Create dispute record
        let dispute = DisputeRecord {
            purchase_id,
            opener: buyer.clone(),
            reason: reason.clone(),
            opened_ledger: current_ledger,
            resolution: DisputeResolution::Unresolved,
            resolved_ledger: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(purchase_id), &dispute);

        // Transition settlement to Disputed
        settlement.state = SettlementState::Disputed;
        settlement.disputed_ledger = Some(current_ledger);
        set_settlement_record(&env, purchase_id, &settlement);

        DisputeOpenedEvent {
            purchase_id,
            material_id: escrow.material_id.clone(),
            opener: buyer,
            reason,
            opened_ledger: current_ledger,
        }
        .publish(&env);

        Ok(())
    }

    /// Resolve an active dispute (admin only).
    ///
    /// - `RefundBuyer`: Refunds the buyer from escrow, revokes entitlement.
    /// - `ReleaseToCreator`: Releases funds to creator, keeps entitlement active.
    /// Transitions settlement from Disputed → Refunded or Disputed → Released.
    pub fn resolve_dispute(
        env: Env,
        admin: Address,
        purchase_id: u64,
        resolution: DisputeResolution,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let dispute: DisputeRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(purchase_id))
            .ok_or(PurchaseError::NoActiveDispute)?;

        // Dispute must not already be resolved
        if dispute.resolution != DisputeResolution::Unresolved {
            return Err(PurchaseError::DisputeNotResolved);
        }

        let mut settlement = get_settlement_record_internal(&env, purchase_id)
            .ok_or(PurchaseError::SettlementNotPending)?;
        if settlement.state != SettlementState::Disputed {
            return Err(PurchaseError::SettlementNotPending);
        }

        let mut escrow =
            get_escrow_record_internal(&env, purchase_id).ok_or(PurchaseError::MaterialNotFound)?;

        let current_ledger = env.ledger().sequence();

        match resolution {
            DisputeResolution::RefundBuyer => {
                // Refund the buyer from the contract's escrowed funds
                if escrow.seller_net > 0 {
                    let contract_address = env.current_contract_address();
                    let buyer = dispute.opener.clone();

                    // Check contract has sufficient balance
                    let balance = SacToken::new(&env, &escrow.asset).balance(&contract_address);
                    if balance < escrow.seller_net {
                        return Err(PurchaseError::InsufficientEscrowBalance);
                    }

                    transfer_asset(
                        &env,
                        &contract_address,
                        &buyer,
                        &escrow.asset,
                        escrow.seller_net,
                    )?;
                }

                // Mark escrow as claimed (funds returned)
                escrow.claimed = true;
                set_escrow_record(&env, purchase_id, &escrow);

                // Revoke entitlement
                let entitlement_key = DataKey::Entitlement((
                    escrow.material_id.clone(),
                    dispute.opener.clone(),
                ));
                if let Some(mut entitlement) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, EntitlementRecord>(&entitlement_key)
                {
                    entitlement.active = false;
                    env.storage()
                        .persistent()
                        .set(&entitlement_key, &entitlement);
                }

                // Update settlement to Refunded
                settlement.state = SettlementState::Refunded;
                settlement.resolved_ledger = Some(current_ledger);
                settlement.refunded_amount = escrow.seller_net;
                set_settlement_record(&env, purchase_id, &settlement);

                // Update dispute record
                let mut updated_dispute = dispute.clone();
                updated_dispute.resolution = DisputeResolution::RefundBuyer;
                updated_dispute.resolved_ledger = Some(current_ledger);
                env.storage()
                    .persistent()
                    .set(&DataKey::Dispute(purchase_id), &updated_dispute);

                PurchaseRefundedEvent {
                    purchase_id,
                    material_id: escrow.material_id.clone(),
                    buyer: dispute.opener.clone(),
                    asset: escrow.asset.clone(),
                    refund_amount: escrow.seller_net,
                    entitlement_revoked: true,
                }
                .publish(&env);
            }
            DisputeResolution::ReleaseToCreator => {
                // Release funds to creator (same as withdraw_payouts)
                if escrow.seller_net > 0 {
                    distribute_payout_shares_from_contract(
                        &env,
                        purchase_id,
                        &escrow.material_id,
                        &escrow,
                    )?;
                }

                escrow.claimed = true;
                set_escrow_record(&env, purchase_id, &escrow);

                // Update settlement to Released
                settlement.state = SettlementState::Released;
                settlement.resolved_ledger = Some(current_ledger);
                set_settlement_record(&env, purchase_id, &settlement);

                // Update dispute record
                let mut updated_dispute = dispute.clone();
                updated_dispute.resolution = DisputeResolution::ReleaseToCreator;
                updated_dispute.resolved_ledger = Some(current_ledger);
                env.storage()
                    .persistent()
                    .set(&DataKey::Dispute(purchase_id), &updated_dispute);

                EscrowReleasedEvent {
                    purchase_id,
                    material_id: escrow.material_id.clone(),
                    asset: escrow.asset.clone(),
                    amount: escrow.seller_net,
                }
                .publish(&env);
            }
            DisputeResolution::Unresolved => {
                // Should never happen because input parameter should not be Unresolved
                return Err(PurchaseError::DisputeNotResolved);
            }
        }

        DisputeResolvedEvent {
            purchase_id,
            material_id: escrow.material_id,
            resolution,
            resolved_ledger: current_ledger,
        }
        .publish(&env);

        Ok(())
    }

    /// Admin-initiated refund for a purchase (without prior dispute).
    ///
    /// Only works when settlement is in Pending state.
    /// Transitions settlement from Pending → Refunded.
    /// Revokes entitlement and returns funds to buyer.
    /// Uses the stored PurchaseBuyer mapping to find the buyer.
    pub fn refund_purchase(
        env: Env,
        admin: Address,
        purchase_id: u64,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let mut settlement = get_settlement_record_internal(&env, purchase_id)
            .ok_or(PurchaseError::SettlementNotPending)?;

        // Only Pending purchases can be refunded (not already released/disputed/refunded)
        if settlement.state != SettlementState::Pending {
            return Err(PurchaseError::RefundNotAllowed);
        }

        let mut escrow =
            get_escrow_record_internal(&env, purchase_id).ok_or(PurchaseError::MaterialNotFound)?;

        if escrow.claimed {
            return Err(PurchaseError::EscrowAlreadyClaimed);
        }

        // Look up the buyer from the PurchaseBuyer mapping
        let buyer: Address = env
            .storage()
            .persistent()
            .get(&DataKey::PurchaseBuyer(purchase_id))
            .ok_or(PurchaseError::NotAuthorized)?;

        let current_ledger = env.ledger().sequence();

        // Verify the buyer has an active entitlement
        let entitlement_key = DataKey::Entitlement((escrow.material_id.clone(), buyer.clone()));
        let entitlement: EntitlementRecord = env
            .storage()
            .persistent()
            .get(&entitlement_key)
            .ok_or(PurchaseError::NotAuthorized)?;

        if !entitlement.active {
            return Err(PurchaseError::NotAuthorized);
        }

        // Refund the buyer from the contract's escrowed funds
        if escrow.seller_net > 0 {
            let contract_address = env.current_contract_address();

            let balance = SacToken::new(&env, &escrow.asset).balance(&contract_address);
            if balance < escrow.seller_net {
                return Err(PurchaseError::InsufficientEscrowBalance);
            }

            transfer_asset(
                &env,
                &contract_address,
                &buyer,
                &escrow.asset,
                escrow.seller_net,
            )?;
        }

        // Mark escrow as claimed
        escrow.claimed = true;
        set_escrow_record(&env, purchase_id, &escrow);

        // Revoke entitlement
        let mut entitlement = entitlement;
        entitlement.active = false;
        env.storage()
            .persistent()
            .set(&entitlement_key, &entitlement);

        // Update settlement to Refunded
        settlement.state = SettlementState::Refunded;
        settlement.resolved_ledger = Some(current_ledger);
        settlement.refunded_amount = escrow.seller_net;
        set_settlement_record(&env, purchase_id, &settlement);

        PurchaseRefundedEvent {
            purchase_id,
            material_id: escrow.material_id.clone(),
            buyer: buyer.clone(),
            asset: escrow.asset.clone(),
            refund_amount: escrow.seller_net,
            entitlement_revoked: true,
        }
        .publish(&env);

        Ok(())
    }

    /// Refund a purchase to a specific buyer (admin only).
    ///
    /// This variant includes the buyer address explicitly for admin-initiated refunds.
    /// Transitions settlement from Pending → Refunded.
    pub fn refund_purchase_to_buyer(
        env: Env,
        admin: Address,
        purchase_id: u64,
        buyer: Address,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let mut settlement = get_settlement_record_internal(&env, purchase_id)
            .ok_or(PurchaseError::SettlementNotPending)?;

        if settlement.state != SettlementState::Pending {
            return Err(PurchaseError::RefundNotAllowed);
        }

        let mut escrow =
            get_escrow_record_internal(&env, purchase_id).ok_or(PurchaseError::MaterialNotFound)?;

        if escrow.claimed {
            return Err(PurchaseError::EscrowAlreadyClaimed);
        }

        let current_ledger = env.ledger().sequence();

        // Verify the buyer has an active entitlement
        let entitlement_key = DataKey::Entitlement((escrow.material_id.clone(), buyer.clone()));
        let entitlement: EntitlementRecord = env
            .storage()
            .persistent()
            .get(&entitlement_key)
            .ok_or(PurchaseError::NotAuthorized)?;

        if !entitlement.active {
            return Err(PurchaseError::NotAuthorized);
        }

        // Refund the buyer from the contract's escrowed funds
        if escrow.seller_net > 0 {
            let contract_address = env.current_contract_address();

            let balance = SacToken::new(&env, &escrow.asset).balance(&contract_address);
            if balance < escrow.seller_net {
                return Err(PurchaseError::InsufficientEscrowBalance);
            }

            transfer_asset(
                &env,
                &contract_address,
                &buyer,
                &escrow.asset,
                escrow.seller_net,
            )?;
        }

        // Mark escrow as claimed
        escrow.claimed = true;
        set_escrow_record(&env, purchase_id, &escrow);

        // Revoke entitlement
        let mut entitlement = entitlement;
        entitlement.active = false;
        env.storage()
            .persistent()
            .set(&entitlement_key, &entitlement);

        // Update settlement to Refunded
        settlement.state = SettlementState::Refunded;
        settlement.resolved_ledger = Some(current_ledger);
        settlement.refunded_amount = escrow.seller_net;
        set_settlement_record(&env, purchase_id, &settlement);

        PurchaseRefundedEvent {
            purchase_id,
            material_id: escrow.material_id.clone(),
            buyer: buyer.clone(),
            asset: escrow.asset.clone(),
            refund_amount: escrow.seller_net,
            entitlement_revoked: true,
        }
        .publish(&env);

        Ok(())
    }

    /// Process a refund for a purchase, returning escrowed funds to the buyer
    /// and revoking their entitlement. Admin-only.
    ///
    /// This is the primary refund entrypoint (`process_refund`). It mirrors
    /// `refund_purchase_to_buyer`: it requires admin authorization, only
    /// operates on `Pending` settlements, rejects already-claimed escrows, and
    /// guarantees that the buyer's token balance increases while the
    /// contract's balance decreases and the entitlement is revoked.
    pub fn process_refund(
        env: Env,
        admin: Address,
        purchase_id: u64,
        buyer: Address,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let mut settlement = get_settlement_record_internal(&env, purchase_id)
            .ok_or(PurchaseError::SettlementNotPending)?;

        if settlement.state != SettlementState::Pending {
            return Err(PurchaseError::RefundNotAllowed);
        }

        let mut escrow =
            get_escrow_record_internal(&env, purchase_id).ok_or(PurchaseError::MaterialNotFound)?;

        if escrow.claimed {
            return Err(PurchaseError::EscrowAlreadyClaimed);
        }

        let current_ledger = env.ledger().sequence();

        let entitlement_key = DataKey::Entitlement((escrow.material_id.clone(), buyer.clone()));
        let entitlement: EntitlementRecord = env
            .storage()
            .persistent()
            .get(&entitlement_key)
            .ok_or(PurchaseError::NotAuthorized)?;

        if !entitlement.active {
            return Err(PurchaseError::NotAuthorized);
        }

        if escrow.seller_net > 0 {
            let contract_address = env.current_contract_address();

            let balance = SacToken::new(&env, &escrow.asset).balance(&contract_address);
            if balance < escrow.seller_net {
                return Err(PurchaseError::InsufficientEscrowBalance);
            }

            transfer_asset(
                &env,
                &contract_address,
                &buyer,
                &escrow.asset,
                escrow.seller_net,
            )?;
        }

        escrow.claimed = true;
        set_escrow_record(&env, purchase_id, &escrow);

        let mut entitlement = entitlement;
        entitlement.active = false;
        env.storage()
            .persistent()
            .set(&entitlement_key, &entitlement);

        settlement.state = SettlementState::Refunded;
        settlement.resolved_ledger = Some(current_ledger);
        settlement.refunded_amount = escrow.seller_net;
        set_settlement_record(&env, purchase_id, &settlement);

        PurchaseRefundedEvent {
            purchase_id,
            material_id: escrow.material_id.clone(),
            buyer: buyer.clone(),
            asset: escrow.asset.clone(),
            refund_amount: escrow.seller_net,
            entitlement_revoked: true,
        }
        .publish(&env);

        Ok(())
    }

    // ============== Settlement Query Functions ==============

    /// Get the settlement record for a purchase
    pub fn get_settlement(env: Env, purchase_id: u64) -> Option<SettlementRecord> {
        get_settlement_record_internal(&env, purchase_id)
    }

    /// Get the dispute record for a purchase
    pub fn get_dispute(env: Env, purchase_id: u64) -> Option<DisputeRecord> {
        env.storage().persistent().get(&DataKey::Dispute(purchase_id))
    }

    /// Get the current settlement state for a purchase
    pub fn get_settlement_state(env: Env, purchase_id: u64) -> Option<SettlementState> {
        get_settlement_record_internal(&env, purchase_id).map(|s| s.state)
    }

    /// Check if a purchase has been settled (reached a terminal state)
    pub fn is_settled(env: Env, purchase_id: u64) -> bool {
        if let Some(settlement) = get_settlement_record_internal(&env, purchase_id) {
            matches!(
                settlement.state,
                SettlementState::Released
                    | SettlementState::Refunded
                    | SettlementState::Expired
            )
        } else {
            false
        }
    }

    /// Check if a purchase has been refunded
    pub fn is_refunded(env: Env, purchase_id: u64) -> bool {
        if let Some(settlement) = get_settlement_record_internal(&env, purchase_id) {
            settlement.state == SettlementState::Refunded
        } else {
            false
        }
    }

    // ============== Admin Functions ==============

    /// Set whether an asset is allowed for purchases (admin only).
    pub fn set_asset_allowed(
        env: Env,
        admin: Address,
        asset: Address,
        kind: AssetKind,
        enabled: bool,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let asset_key = DataKey::AllowedAsset(asset.clone());
        let is_new_asset = !env.storage().persistent().has(&asset_key);

        let info = AssetInfo { kind, enabled };
        env.storage().persistent().set(&asset_key, &info);
        extend_persistent_ttl(&env, &asset_key);

        if is_new_asset {
            index_allowed_asset(&env, &asset);
        }

        AssetPolicyUpdatedEvent {
            asset,
            kind,
            enabled,
        }
        .publish(&env);

        Ok(())
    }

    /// Update platform configuration (admin only)
    pub fn set_platform_config(
        env: Env,
        admin: Address,
        treasury: Address,
        platform_fee_bps: u32,
        paused: bool,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        if platform_fee_bps > MAX_PLATFORM_FEE_BPS {
            return Err(PurchaseError::InvalidPlatformFee);
        }

        if treasury == env.current_contract_address() {
            return Err(PurchaseError::InvalidTreasury);
        }

        let current_config = get_platform_config(&env)?;

        let new_config = PlatformConfig {
            registry: current_config.registry,
            treasury: treasury.clone(),
            platform_fee_bps,
            paused,
            oracle: current_config.oracle,
        };

        put_platform_config(&env, &new_config);

        PlatformConfigUpdatedEvent {
            treasury,
            platform_fee_bps,
            paused,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the full `AssetInfo` record for `asset`, if present.
    pub fn get_asset_info(env: Env, asset: Address) -> Option<AssetInfo> {
        let key = DataKey::AllowedAsset(asset);
        let info = env.storage().persistent().get(&key);
        if info.is_some() {
            extend_persistent_ttl(&env, &key);
        }
        info
    }

    /// Configure the price-oracle address (admin only).
    pub fn set_oracle(
        env: Env,
        admin: Address,
        oracle: Option<Address>,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let mut config = get_platform_config(&env)?;
        config.oracle = oracle;
        put_platform_config(&env, &config);

        Ok(())
    }

    /// Update registry address (admin only, for migrations)
    pub fn set_registry(env: Env, admin: Address, registry: Address) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let mut config = get_platform_config(&env)?;
        config.registry = registry;

        put_platform_config(&env, &config);

        Ok(())
    }

    /// Update the platform fee rate (admin only).
    pub fn update_platform_fee(
        env: Env,
        admin: Address,
        new_platform_fee_bps: u32,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        if new_platform_fee_bps > MAX_PLATFORM_FEE_BPS {
            return Err(PurchaseError::InvalidPlatformFee);
        }

        let mut config = get_platform_config(&env)?;
        config.platform_fee_bps = new_platform_fee_bps;

        put_platform_config(&env, &config);

        PlatformConfigUpdatedEvent {
            treasury: config.treasury.clone(),
            platform_fee_bps: new_platform_fee_bps,
            paused: config.paused,
        }
        .publish(&env);

        Ok(())
    }

    /// Pause contract operations (admin only)
    pub fn pause(env: Env, admin: Address) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let mut config = get_platform_config(&env)?;
        config.paused = true;

        put_platform_config(&env, &config);

        PlatformConfigUpdatedEvent {
            treasury: config.treasury,
            platform_fee_bps: config.platform_fee_bps,
            paused: true,
        }
        .publish(&env);

        Ok(())
    }

    /// Unpause contract operations (admin only)
    pub fn unpause(env: Env, admin: Address) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let mut config = get_platform_config(&env)?;
        config.paused = false;

        put_platform_config(&env, &config);

        PlatformConfigUpdatedEvent {
            treasury: config.treasury,
            platform_fee_bps: config.platform_fee_bps,
            paused: false,
        }
        .publish(&env);

        Ok(())
    }

    /// Upgrade contract WASM hash (admin only).
    pub fn upgrade(
        env: Env,
        admin: Address,
        new_wasm_hash: BytesN<32>,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        Ok(())
    }

    // ============== Admin Transfer (two-step) ==============

    /// Begin an admin ownership transfer.
    pub fn transfer_admin(
        env: Env,
        admin: Address,
        new_admin: Address,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        env.storage().instance().set(&DataKey::PendingAdmin, &new_admin);
        extend_instance_ttl(&env);

        AdminTransferInitiatedEvent {
            from: admin,
            pending_admin: new_admin,
        }
        .publish(&env);

        Ok(())
    }

    /// Complete an admin ownership transfer initiated by `transfer_admin`.
    pub fn accept_admin(env: Env, new_admin: Address) -> Result<(), PurchaseError> {
        new_admin.require_auth();

        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(PurchaseError::NoPendingAdminTransfer)?;
        extend_instance_ttl(&env);

        if pending != new_admin {
            return Err(PurchaseError::NotAuthorized);
        }

        auth::set_admin_role(&env, &new_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        extend_instance_ttl(&env);

        AdminTransferAcceptedEvent {
            new_admin: new_admin.clone(),
        }
        .publish(&env);

        Ok(())
    }

    /// Return the pending admin address, if a transfer is in progress.
    pub fn get_pending_admin(env: Env) -> Option<Address> {
        let pending = env.storage().instance().get(&DataKey::PendingAdmin);
        extend_instance_ttl(&env);
        pending
    }

    /// Return the buyer address for a purchase, if known.
    pub fn get_purchase_buyer(env: Env, purchase_id: u64) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::PurchaseBuyer(purchase_id))
    }

    // ============== Creator Volume Tiers ==============

    /// Assign a volume tier to a creator (admin only).
    pub fn set_creator_tier(
        env: Env,
        admin: Address,
        creator: Address,
        tier: CreatorTier,
    ) -> Result<(), PurchaseError> {
        auth::require_admin(&env, &admin)?;

        let tier_key = DataKey::CreatorTier(creator.clone());
        let is_new_creator = !env.storage().persistent().has(&tier_key);

        env.storage().persistent().set(&tier_key, &tier);
        extend_persistent_ttl(&env, &tier_key);

        if is_new_creator {
            index_creator_tier(&env, &creator);
        }

        CreatorTierUpdatedEvent {
            creator,
            tier,
        }
        .publish(&env);

        Ok(())
    }

    /// Return the volume tier assigned to a creator.
    pub fn get_creator_tier(env: Env, creator: Address) -> CreatorTier {
        let key = DataKey::CreatorTier(creator);
        let tier = env.storage().persistent().get(&key);
        if tier.is_some() {
            extend_persistent_ttl(&env, &key);
        }
        tier.unwrap_or(CreatorTier::Default)
    }

    // ============== TTL Maintenance (#464) ==============

    /// Bump the TTL of up to `limit` (capped at `MAX_MAINTENANCE_BATCH`)
    /// purchases — both the `Escrow` and `Entitlement` halves together —
    /// starting at `cursor`. Permissionless, cursor-based; this is the
    /// entrypoint that specifically protects paid access and escrowed
    /// funds, which otherwise have no natural heartbeat once a buyer stops
    /// re-checking their entitlement.
    ///
    /// Returns the cursor to resume from. Skips (rather than aborts on) any
    /// slot whose escrow/entitlement has already expired/archived — see
    /// ../../docs/ttl-operations.md for the restoration runbook.
    pub fn extend_purchases_ttl(env: Env, cursor: u64, limit: u32) -> u64 {
        let limit = (limit.min(MAX_MAINTENANCE_BATCH)) as u64;
        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::PurchaseIndexCount)
            .unwrap_or(0);
        extend_instance_ttl(&env);

        let end = cursor.saturating_add(limit).min(count);
        let mut i = cursor;
        while i < end {
            let index_key = DataKey::PurchaseIndex(i);
            if let Some((purchase_id, material_id, buyer)) = env
                .storage()
                .persistent()
                .get::<_, (u64, BytesN<32>, Address)>(&index_key)
            {
                extend_persistent_ttl(&env, &index_key);

                let escrow_key = DataKey::Escrow(purchase_id);
                if env.storage().persistent().has(&escrow_key) {
                    extend_persistent_ttl(&env, &escrow_key);
                }

                let entitlement_key = DataKey::Entitlement((material_id, buyer));
                if env.storage().persistent().has(&entitlement_key) {
                    extend_persistent_ttl(&env, &entitlement_key);
                }
            }
            i += 1;
        }

        end
    }

    /// Bump the TTL of up to `limit` allowlisted assets, starting at
    /// `cursor`. Same semantics as `extend_purchases_ttl`.
    pub fn extend_allowed_asset_ttl(env: Env, cursor: u64, limit: u32) -> u64 {
        let limit = (limit.min(MAX_MAINTENANCE_BATCH)) as u64;
        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::AllowedAssetIndexCount)
            .unwrap_or(0);
        extend_instance_ttl(&env);

        let end = cursor.saturating_add(limit).min(count);
        let mut i = cursor;
        while i < end {
            let index_key = DataKey::AllowedAssetIndex(i);
            if let Some(asset) = env.storage().persistent().get::<_, Address>(&index_key) {
                extend_persistent_ttl(&env, &index_key);
                let asset_key = DataKey::AllowedAsset(asset);
                if env.storage().persistent().has(&asset_key) {
                    extend_persistent_ttl(&env, &asset_key);
                }
            }
            i += 1;
        }

        end
    }

    /// Bump the TTL of up to `limit` creator-tier assignments, starting at
    /// `cursor`. Same semantics as `extend_purchases_ttl`.
    pub fn extend_creator_tier_ttl(env: Env, cursor: u64, limit: u32) -> u64 {
        let limit = (limit.min(MAX_MAINTENANCE_BATCH)) as u64;
        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::CreatorTierIndexCount)
            .unwrap_or(0);
        extend_instance_ttl(&env);

        let end = cursor.saturating_add(limit).min(count);
        let mut i = cursor;
        while i < end {
            let index_key = DataKey::CreatorTierIndex(i);
            if let Some(creator) = env.storage().persistent().get::<_, Address>(&index_key) {
                extend_persistent_ttl(&env, &index_key);
                let tier_key = DataKey::CreatorTier(creator);
                if env.storage().persistent().has(&tier_key) {
                    extend_persistent_ttl(&env, &tier_key);
                }
            }
            i += 1;
        }

        end
    }

    /// Bump the TTL of up to `limit` admin-role grants, starting at
    /// `cursor`. Same semantics as `extend_purchases_ttl`; delegates to
    /// `auth`, which owns the `AdminRole` storage namespace.
    pub fn extend_admin_role_ttl(env: Env, cursor: u64, limit: u32) -> u64 {
        auth::extend_admin_role_ttl(&env, cursor, limit)
    }
}

// ============== TTL Renewal (#464) ==============

/// Extends `key`'s persistent-storage TTL back out to the network maximum
/// whenever it has dropped below half of that maximum. Safe and cheap to
/// call on every read and write of a tracked key — a no-op when the TTL is
/// already healthy. `pub(crate)` so `auth.rs` can reuse it for `AdminRole`.
pub(crate) fn extend_persistent_ttl<K: IntoVal<Env, Val>>(env: &Env, key: &K) {
    let max_ttl = env.storage().max_ttl();
    env.storage()
        .persistent()
        .extend_ttl(key, max_ttl / TTL_RENEWAL_DIVISOR, max_ttl);
}

/// Extends the contract instance's TTL — and therefore everything stored in
/// instance storage alongside it (`PlatformConfig`, `PurchaseNonce`,
/// `PendingAdmin`, the maintenance counters) — back out to the network
/// maximum whenever it has dropped below half of that maximum.
pub(crate) fn extend_instance_ttl(env: &Env) {
    let max_ttl = env.storage().max_ttl();
    env.storage()
        .instance()
        .extend_ttl(max_ttl / TTL_RENEWAL_DIVISOR, max_ttl);
}

/// Appends `(purchase_id, material_id, buyer)` to the purchase maintenance
/// index and bumps the instance-stored count, so `extend_purchases_ttl` can
/// renew both the `Escrow` and `Entitlement` halves of every purchase
/// without off-chain enumeration.
fn index_purchase(env: &Env, purchase_id: u64, material_id: &BytesN<32>, buyer: &Address) {
    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::PurchaseIndexCount)
        .unwrap_or(0);
    let index_key = DataKey::PurchaseIndex(count);
    let entry = (purchase_id, material_id.clone(), buyer.clone());
    env.storage().persistent().set(&index_key, &entry);
    extend_persistent_ttl(env, &index_key);
    env.storage()
        .instance()
        .set(&DataKey::PurchaseIndexCount, &(count + 1));
    extend_instance_ttl(env);
}

/// Appends `asset` to the allowlist maintenance index. Only called the
/// first time an asset is added to the allowlist.
fn index_allowed_asset(env: &Env, asset: &Address) {
    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::AllowedAssetIndexCount)
        .unwrap_or(0);
    let index_key = DataKey::AllowedAssetIndex(count);
    env.storage().persistent().set(&index_key, asset);
    extend_persistent_ttl(env, &index_key);
    env.storage()
        .instance()
        .set(&DataKey::AllowedAssetIndexCount, &(count + 1));
    extend_instance_ttl(env);
}

/// Appends `creator` to the creator-tier maintenance index. Only called the
/// first time a tier is assigned to that creator.
fn index_creator_tier(env: &Env, creator: &Address) {
    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::CreatorTierIndexCount)
        .unwrap_or(0);
    let index_key = DataKey::CreatorTierIndex(count);
    env.storage().persistent().set(&index_key, creator);
    extend_persistent_ttl(env, &index_key);
    env.storage()
        .instance()
        .set(&DataKey::CreatorTierIndexCount, &(count + 1));
    extend_instance_ttl(env);
}

// ============== Internal Functions ==============

/// Return the effective platform fee rate for `creator`.
fn get_effective_fee_bps(env: &Env, creator: &Address, config_fee_bps: u32) -> u32 {
    let key = DataKey::CreatorTier(creator.clone());
    let tier: Option<CreatorTier> = env.storage().persistent().get(&key);
    if tier.is_some() {
        extend_persistent_ttl(env, &key);
    }
    match tier.unwrap_or(CreatorTier::Default) {
        CreatorTier::Default => config_fee_bps,
        CreatorTier::Tier1 => TIER1_FEE_BPS,
        CreatorTier::Tier2 => TIER2_FEE_BPS,
    }
}

fn get_platform_config(env: &Env) -> Result<PlatformConfig, PurchaseError> {
    let config = env
        .storage()
        .instance()
        .get(&DataKey::PlatformConfig)
        .ok_or(PurchaseError::NotAuthorized)?;
    extend_instance_ttl(env);
    Ok(config)
}

fn put_platform_config(env: &Env, config: &PlatformConfig) {
    env.storage().instance().set(&DataKey::PlatformConfig, config);
    extend_instance_ttl(env);
}

fn is_asset_allowed(env: &Env, asset: &Address) -> bool {
    let key = DataKey::AllowedAsset(asset.clone());
    let info: Option<AssetInfo> = env.storage().persistent().get(&key);
    if info.is_some() {
        extend_persistent_ttl(env, &key);
    }
    info.map(|info| info.enabled).unwrap_or(false)
}

fn find_quote(quotes: &Vec<AssetQuote>, asset: &Address) -> Option<AssetQuote> {
    let mut index = 0;
    while index < quotes.len() {
        let quote = quotes.get_unchecked(index);
        if quote.asset == *asset {
            return Some(quote);
        }
        index += 1;
    }
    None
}

fn get_and_increment_purchase_nonce(env: &Env) -> Result<u64, PurchaseError> {
    let nonce: u64 = env
        .storage()
        .instance()
        .get(&DataKey::PurchaseNonce)
        .ok_or(PurchaseError::NotAuthorized)?;
    env.storage()
        .instance()
        .set(&DataKey::PurchaseNonce, &(nonce + 1));
    extend_instance_ttl(env);
    Ok(nonce)
}

fn transfer_asset(
    env: &Env,
    from: &Address,
    to: &Address,
    asset: &Address,
    amount: i128,
) -> Result<(), PurchaseError> {
    SacToken::new(env, asset).transfer(from, to, amount);
    Ok(())
}

fn validate_payout_shares(payout_shares: &Vec<PayoutShare>) -> Result<(), PurchaseError> {
    let share_count = payout_shares.len();
    if share_count == 0 || share_count > MAX_PAYOUT_RECIPIENTS {
        return Err(PurchaseError::InvalidPayoutShares);
    }

    let mut total_share_bps = 0u32;
    let mut index = 0;
    while index < share_count {
        let share = payout_shares.get_unchecked(index);
        if share.share_bps == 0 || share.share_bps > BASIS_POINTS {
            return Err(PurchaseError::InvalidPayoutShares);
        }

        total_share_bps = total_share_bps
            .checked_add(share.share_bps)
            .ok_or(PurchaseError::InvalidPayoutShares)?;

        let mut other = index + 1;
        while other < share_count {
            if share.recipient == payout_shares.get_unchecked(other).recipient {
                return Err(PurchaseError::InvalidPayoutShares);
            }
            other += 1;
        }

        index += 1;
    }

    if total_share_bps != BASIS_POINTS {
        return Err(PurchaseError::InvalidPayoutShares);
    }

    Ok(())
}

fn has_entitlement_internal(env: &Env, material_id: &BytesN<32>, buyer: &Address) -> bool {
    get_entitlement_internal(env, material_id, buyer)
        .map(|e| e.active)
        .unwrap_or(false)
}

/// Reads an entitlement and, when present, extends its TTL — the primary
/// organic renewal path for paid access (#464 Tier D): a buyer actively
/// using what they paid for keeps their own record alive for free, every
/// time a content-access check runs.
fn get_entitlement_internal(
    env: &Env,
    material_id: &BytesN<32>,
    buyer: &Address,
) -> Option<EntitlementRecord> {
    let key = DataKey::Entitlement((material_id.clone(), buyer.clone()));
    let entitlement = env.storage().persistent().get(&key);
    if entitlement.is_some() {
        extend_persistent_ttl(env, &key);
    }
    entitlement
}

fn set_entitlement(env: &Env, entitlement: &EntitlementRecord) {
    let key = DataKey::Entitlement((entitlement.material_id.clone(), entitlement.buyer.clone()));
    env.storage().persistent().set(&key, entitlement);
    extend_persistent_ttl(env, &key);
}

fn get_escrow_record_internal(env: &Env, purchase_id: u64) -> Option<EscrowRecord> {
    let key = DataKey::Escrow(purchase_id);
    let escrow = env.storage().persistent().get(&key);
    if escrow.is_some() {
        extend_persistent_ttl(env, &key);
    }
    escrow
}

fn set_escrow_record(env: &Env, purchase_id: u64, record: &EscrowRecord) {
    let key = DataKey::Escrow(purchase_id);
    env.storage().persistent().set(&key, record);
    extend_persistent_ttl(env, &key);
}

fn get_settlement_record_internal(env: &Env, purchase_id: u64) -> Option<SettlementRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::Settlement(purchase_id))
}

fn set_settlement_record(env: &Env, purchase_id: u64, record: &SettlementRecord) {
    env.storage()
        .persistent()
        .set(&DataKey::Settlement(purchase_id), record);
}

fn distribute_payout_shares_from_contract(
    env: &Env,
    purchase_id: u64,
    material_id: &BytesN<32>,
    escrow: &EscrowRecord,
) -> Result<(), PurchaseError> {
    let contract_address = env.current_contract_address();
    let mut total_distributed: i128 = 0;
    let share_count = escrow.payout_shares.len();

    let mut index = 0;
    while index < share_count {
        let share = escrow.payout_shares.get_unchecked(index);

        let share_amount = if index == share_count - 1 {
            escrow.seller_net - total_distributed
        } else {
            (escrow.seller_net * share.share_bps as i128) / BASIS_POINTS as i128
        };

        if share_amount > 0 {
            transfer_asset(
                env,
                &contract_address,
                &share.recipient,
                &escrow.asset,
                share_amount,
            )?;
            total_distributed += share_amount;

            PayoutDistributedEvent {
                purchase_id,
                material_id: material_id.clone(),
                recipient: share.recipient.clone(),
                role: Symbol::new(env, "creator_share"),
                asset: escrow.asset.clone(),
                amount: share_amount,
                // EscrowRecord does not carry the originating purchase's
                // transaction_id, so the delayed-release payout event can't
                // correlate back to it — empty Bytes is an accepted value
                // for this field (see `empty_transaction_id_is_accepted`).
                transaction_id: Bytes::new(env),
            }
            .publish(env);
        }

        index += 1;
    }

    if total_distributed != escrow.seller_net {
        return Err(PurchaseError::InvalidPayoutShares);
    }

    Ok(())
}

/// Verify that `caller` is the current admin (used by functions that call
/// `require_auth` before this check).
fn verify_admin(env: &Env, caller: &Address) -> Result<(), PurchaseError> {
    if !auth::has_admin_role(env, caller) {
        return Err(PurchaseError::NotAuthorized);
    }
    Ok(())
}

#[cfg(test)]
mod test;