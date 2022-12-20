use mina_hasher::Fp;
use mina_signer::CompressedPubKey;

use crate::{
    scan_state::{currency::Magnitude, transaction_logic::transaction_applied::Varying},
    Account, AccountId, Address, BaseLedger, GetOrCreated, ReceiptChainHash, Timing, TokenId,
};

use self::{
    protocol_state::ProtocolStateView,
    signed_command::{SignedCommand, SignedCommandPayload},
    transaction_applied::TransactionApplied,
    transaction_union_payload::TransactionUnionPayload,
    zkapp_command::AccountNonce,
};

use super::{
    currency::{Amount, Balance, Fee},
    scan_state::{transaction_snark::OneOrTwo, ConstraintConstants},
};

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/transaction_status.ml#L9
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub enum TransactionFailure {
    Predicate,
    Source_not_present,
    Receiver_not_present,
    Amount_insufficient_to_create_account,
    Cannot_pay_creation_fee_in_token,
    Source_insufficient_balance,
    Source_minimum_balance_violation,
    Receiver_already_exists,
    Token_owner_not_caller,
    Overflow,
    Global_excess_overflow,
    Local_excess_overflow,
    Local_supply_increase_overflow,
    Global_supply_increase_overflow,
    Signed_command_on_zkapp_account,
    Zkapp_account_not_present,
    Update_not_permitted_balance,
    Update_not_permitted_timing_existing_account,
    Update_not_permitted_delegate,
    Update_not_permitted_app_state,
    Update_not_permitted_verification_key,
    Update_not_permitted_sequence_state,
    Update_not_permitted_zkapp_uri,
    Update_not_permitted_token_symbol,
    Update_not_permitted_permissions,
    Update_not_permitted_nonce,
    Update_not_permitted_voting_for,
    Zkapp_command_replay_check_failed,
    Fee_payer_nonce_must_increase,
    Fee_payer_must_be_signed,
    Account_balance_precondition_unsatisfied,
    Account_nonce_precondition_unsatisfied,
    Account_receipt_chain_hash_precondition_unsatisfied,
    Account_delegate_precondition_unsatisfied,
    Account_sequence_state_precondition_unsatisfied,
    Account_app_state_precondition_unsatisfied(i64),
    Account_proved_state_precondition_unsatisfied,
    Account_is_new_precondition_unsatisfied,
    Protocol_state_precondition_unsatisfied,
    Incorrect_nonce,
    Invalid_fee_excess,
    Cancelled,
}

pub fn single_failure() -> Vec<Vec<TransactionFailure>> {
    vec![vec![TransactionFailure::Update_not_permitted_balance]]
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/transaction_status.ml#L452
#[derive(Debug, Clone)]
pub enum TransactionStatus {
    Applied,
    Failed(Vec<Vec<TransactionFailure>>),
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/with_status.ml#L6
#[derive(Debug, Clone)]
pub struct WithStatus<T> {
    pub data: T,
    pub status: TransactionStatus,
}

impl<T> WithStatus<T> {
    pub fn map<F, R>(&self, fun: F) -> WithStatus<R>
    where
        F: Fn(&T) -> R,
    {
        WithStatus {
            data: fun(&self.data),
            status: self.status.clone(),
        }
    }
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/fee_transfer.ml#L19
#[derive(Debug, Clone)]
pub struct FeeTransferInner {
    receiver_pk: CompressedPubKey,
    fee: Fee,
    fee_token: TokenId,
}

impl FeeTransferInner {
    pub fn receiver(&self) -> AccountId {
        AccountId {
            public_key: self.receiver_pk.clone(),
            token_id: self.fee_token.clone(),
        }
    }
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/fee_transfer.ml#L68
#[derive(Debug, Clone)]
pub struct FeeTransfer(OneOrTwo<FeeTransferInner>);

impl std::ops::Deref for FeeTransfer {
    type Target = OneOrTwo<FeeTransferInner>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FeeTransfer {
    pub fn fee_tokens(&self) -> impl Iterator<Item = &TokenId> {
        self.0.iter().map(|fee_transfer| &fee_transfer.fee_token)
    }

    pub fn receiver_pks(&self) -> impl Iterator<Item = &CompressedPubKey> {
        self.0.iter().map(|fee_transfer| &fee_transfer.receiver_pk)
    }

    pub fn receivers(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.0.iter().map(|fee_transfer| AccountId {
            public_key: fee_transfer.receiver_pk.clone(),
            token_id: fee_transfer.fee_token.clone(),
        })
    }
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/coinbase.ml#L17
#[derive(Debug, Clone)]
pub struct Coinbase {
    receiver: CompressedPubKey,
    amount: Amount,
    fee_transfer: FeeTransfer,
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/signature.mli#L11
#[derive(Debug, Clone)]
pub struct Signature((Fp, (Fp, Fp)));

pub type Memo = Vec<u8>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slot(pub(super) u32);

impl Slot {
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct Index(pub(super) u32);

pub mod signed_command {
    use crate::AccountId;

    use super::{zkapp_command::AccountNonce, *};

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/signed_command_payload.ml#L75
    #[derive(Debug, Clone)]
    pub struct Common {
        pub fee: Fee,
        pub fee_token: TokenId,
        pub fee_payer_pk: CompressedPubKey,
        pub nonce: AccountNonce,
        pub valid_until: Slot,
        pub memo: Memo,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/payment_payload.ml#L40
    #[derive(Debug, Clone)]
    pub struct PaymentPayload {
        pub source_pk: CompressedPubKey,
        pub receiver_pk: CompressedPubKey,
        pub amount: Amount,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/stake_delegation.ml#L11
    #[derive(Debug, Clone)]
    pub enum StakeDelegationPayload {
        SetDelegate {
            delegator: CompressedPubKey,
            new_delegate: CompressedPubKey,
        },
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/signed_command_payload.mli#L24
    #[derive(Debug, Clone)]
    pub enum Body {
        Payment(PaymentPayload),
        StakeDelegation(StakeDelegationPayload),
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/signed_command_payload.mli#L165
    #[derive(Debug, Clone)]
    pub struct SignedCommandPayload {
        pub common: Common,
        pub body: Body,
    }

    #[derive(Debug, Clone)]
    pub struct SignedCommand {
        pub payload: SignedCommandPayload,
        pub signer: CompressedPubKey,
        pub signature: Signature,
    }

    impl SignedCommand {
        pub fn valid_until(&self) -> Slot {
            self.payload.common.valid_until.clone()
        }

        /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/signed_command_payload.ml#L322
        pub fn fee_payer(&self) -> AccountId {
            let public_key = self.payload.common.fee_payer_pk.clone();
            AccountId::new(public_key, TokenId::default())
        }

        /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/signed_command_payload.ml#L318
        pub fn fee_token(&self) -> TokenId {
            TokenId::default()
        }

        pub fn fee(&self) -> Fee {
            self.payload.common.fee.clone()
        }

        pub fn nonce(&self) -> AccountNonce {
            self.payload.common.nonce.clone()
        }
    }
}

pub mod zkapp_command {
    use crate::{
        scan_state::currency::{Balance, Signed},
        AuthRequired, Permissions, Slot, Timing, TokenSymbol, VerificationKey,
    };

    use super::*;

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L834
    #[derive(Debug, Clone)]
    pub struct Events(Vec<Vec<Fp>>);

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_basic.ml#L100
    #[derive(Debug, Clone)]
    pub enum SetOrKeep<T> {
        Set(T),
        Kepp,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L319
    #[derive(Debug, Clone)]
    pub struct Update {
        app_state: [SetOrKeep<Fp>; 8],
        delegate: SetOrKeep<CompressedPubKey>,
        verification_key: SetOrKeep<VerificationKey>,
        permissions: SetOrKeep<Permissions<AuthRequired>>,
        zkapp_uri: SetOrKeep<String>,
        token_symbol: SetOrKeep<TokenSymbol>,
        timing: SetOrKeep<Timing>,
        voting_for: SetOrKeep<Fp>,
    }

    // TODO: This could be std::ops::Range ?
    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_precondition.ml#L23
    #[derive(Debug, Clone)]
    pub struct ClosedInterval<T> {
        lower: T,
        upper: T,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_basic.ml#L232
    #[derive(Debug, Clone)]
    pub enum OrIgnore<T> {
        Check(T),
        Ignore,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_precondition.ml#L439
    pub type Hash<T> = OrIgnore<T>;

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_precondition.ml#L298
    pub type EqData<T> = OrIgnore<T>;

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_precondition.ml#L178
    pub type Numeric<T> = OrIgnore<ClosedInterval<T>>;

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/block_time/intf.ml#L55
    // TODO: Not sure if it's `u64`, but OCaml has methods `of_int64` and `to_in64`
    #[derive(Debug, Clone)]
    pub struct BlockTime(pub(super) u64);

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_numbers/length.mli#L2
    #[derive(Debug, Clone)]
    pub struct Length(pub(super) u32);

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/epoch_ledger.ml#L9
    #[derive(Debug, Clone)]
    pub struct EpochLedger {
        hash: Hash<Fp>,
        total_currency: Numeric<Amount>,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_precondition.ml#L797
    #[derive(Debug, Clone)]
    pub struct EpochData {
        ledger: EpochLedger,
        seed: Hash<Fp>,
        start_checkpoint: Hash<Fp>,
        lock_checkpoint: Hash<Fp>,
        epoch_length: Numeric<Length>,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_precondition.ml#L977
    #[derive(Debug, Clone)]
    pub struct ZkAppPreconditions {
        snarked_ledger_hash: Hash<Fp>,
        timestamp: Numeric<BlockTime>,
        blockchain_length: Numeric<Length>,
        min_window_density: Numeric<Length>,
        last_vrf_output: (), // It's not defined in OCAml
        total_currency: Numeric<Amount>,
        global_slot_since_hard_fork: Numeric<Slot>,
        global_slot_since_genesis: Numeric<Slot>,
        staking_epoch_data: EpochData,
        next_epoch_data: EpochData,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_numbers/account_nonce.mli#L2
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct AccountNonce(pub(super) u32);

    impl AccountNonce {
        pub fn is_zero(&self) -> bool {
            self.0 == 0
        }

        pub fn as_u32(&self) -> u32 {
            self.0
        }
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_precondition.ml#L478
    #[derive(Debug, Clone)]
    pub struct Account {
        balance: Numeric<Balance>,
        nonce: Numeric<AccountNonce>,
        receipt_chain_hash: Hash<Fp>, // TODO: Should be type `ReceiptChainHash`
        delegate: EqData<CompressedPubKey>,
        state: [EqData<Fp>; 8],
        sequence_state: EqData<Fp>,
        proved_state: EqData<bool>,
        is_new: EqData<bool>,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L613
    #[derive(Debug, Clone)]
    pub enum AccountPreconditions {
        Full(Box<Account>),
        Nonce(AccountNonce),
        Accept,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L758
    #[derive(Debug, Clone)]
    pub struct Preconditions {
        network: ZkAppPreconditions,
        account: AccountPreconditions,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L27
    #[derive(Debug, Clone)]
    pub enum AuthorizationKind {
        NoneGiven,
        Signature,
        Proof,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L955
    #[derive(Debug, Clone)]
    pub struct Body {
        public_key: CompressedPubKey,
        token_id: TokenId,
        update: Update,
        balance_change: Signed<Amount>,
        increment_nonce: bool,
        events: Events,
        sequence_events: Events,
        call_data: Fp,
        preconditions: Preconditions,
        use_full_commitment: bool,
        caller: TokenId,
        authorization_kind: AuthorizationKind,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/pickles/pickles_intf.ml#L316
    #[derive(Debug, Clone)]
    pub struct SideLoadedProof {
        // Side_loaded.Proof
        // TODO: Not sure what type this is yet...
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/control.ml#L11
    #[derive(Debug, Clone)]
    pub enum Control {
        Proof(SideLoadedProof),
        Signature(Signature),
        NoneGiven,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L1437
    #[derive(Debug, Clone)]
    pub struct AccountUpdate {
        body: Body,
        authorization: Control,
    }

    // Digest.Account_update.Stable.V1.t = Fp
    // Digest.Forest.Stable.V1.t = Fp

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_command.ml#L49
    #[derive(Debug, Clone)]
    pub struct Tree {
        account_update: AccountUpdate,
        account_update_digest: Fp,
        calls: Vec<WithStackHash>,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/with_stack_hash.ml#L6
    #[derive(Debug, Clone)]
    pub struct WithStackHash {
        elt: Tree,
        pub stack_hash: Fp,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_command.ml#L345
    #[derive(Debug, Clone)]
    pub struct CallForest(pub Vec<WithStackHash>);

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L1081
    #[derive(Debug, Clone)]
    pub struct FeePayerBody {
        public_key: CompressedPubKey,
        fee: Fee,
        valid_until: Option<Slot>,
        nonce: AccountNonce,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/account_update.ml#L1484
    #[derive(Debug, Clone)]
    pub struct FeePayer {
        body: FeePayerBody,
        authorization: Signature,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/zkapp_command.ml#L959
    #[derive(Debug, Clone)]
    pub struct ZkAppCommand {
        fee_payer: FeePayer,
        account_updates: CallForest,
        memo: Memo,
    }
}

pub enum UserCommand {
    SignedCommand(Box<signed_command::SignedCommand>),
    ZkAppCommand(Box<zkapp_command::ZkAppCommand>),
}

pub enum Transaction {
    Command(UserCommand),
    FeeTransfer(FeeTransfer),
    Coinbase(Coinbase),
}

pub mod transaction_applied {
    use crate::{Account, AccountId};

    use super::*;

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L17
    #[derive(Debug, Clone)]
    pub struct SignedCommandApplied {
        user_command: WithStatus<signed_command::SignedCommand>,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L65
    #[derive(Debug, Clone)]
    pub struct ZkappCommandApplied {
        accounts: Vec<(AccountId, Option<Account>)>,
        command: WithStatus<zkapp_command::ZkAppCommand>,
        new_accounts: Vec<AccountId>,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L82
    #[derive(Debug, Clone)]
    pub enum CommandApplied {
        SignedCommand(Box<SignedCommandApplied>),
        ZkappCommand(Box<ZkappCommandApplied>),
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L96
    #[derive(Debug, Clone)]
    pub struct FeeTransferApplied {
        pub fee_transfer: WithStatus<FeeTransfer>,
        pub new_accounts: Vec<AccountId>,
        pub burned_tokens: Amount,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L112
    #[derive(Debug, Clone)]
    pub struct CoinbaseApplied {
        coinbase: WithStatus<Coinbase>,
        new_accounts: Vec<AccountId>,
        burned_tokens: Amount,
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L142
    #[derive(Debug, Clone)]
    pub enum Varying {
        Command(CommandApplied),
        FeeTransfer(FeeTransferApplied),
        Coinbase(CoinbaseApplied),
    }

    /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L142
    #[derive(Debug, Clone)]
    pub struct TransactionApplied {
        pub previous_hash: Fp,
        pub varying: Varying,
    }

    impl TransactionApplied {
        /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L639
        pub fn transaction(&self) -> WithStatus<Transaction> {
            use CommandApplied::*;
            use Varying::*;

            match &self.varying {
                Command(SignedCommand(cmd)) => cmd
                    .user_command
                    .map(|c| Transaction::Command(UserCommand::SignedCommand(Box::new(c.clone())))),
                Command(ZkappCommand(cmd)) => cmd
                    .command
                    .map(|c| Transaction::Command(UserCommand::ZkAppCommand(Box::new(c.clone())))),
                FeeTransfer(f) => f.fee_transfer.map(|f| Transaction::FeeTransfer(f.clone())),
                Coinbase(c) => c.coinbase.map(|c| Transaction::Coinbase(c.clone())),
            }
        }
    }
}

pub mod protocol_state {
    use mina_p2p_messages::v2::MinaStateProtocolStateValueStableV2;

    use super::{
        zkapp_command::{BlockTime, Length},
        *,
    };

    #[derive(Debug, Clone)]
    pub struct EpochLedger {
        hash: Fp,
        total_currency: Amount,
    }

    #[derive(Debug, Clone)]
    pub struct EpochData {
        ledger: EpochLedger,
        seed: Fp,
        start_checkpoint: Fp,
        lock_checkpoint: Fp,
        epoch_length: Length,
    }

    #[derive(Debug, Clone)]
    pub struct ProtocolStateView {
        pub snarked_ledger_hash: Fp,
        pub timestamp: BlockTime,
        pub blockchain_length: Length,
        pub min_window_density: Length,
        pub last_vrf_output: (), // It's not defined in OCAml
        pub total_currency: Amount,
        pub global_slot_since_hard_fork: Slot,
        pub global_slot_since_genesis: Slot,
        pub staking_epoch_data: EpochData,
        pub next_epoch_data: EpochData,
    }

    pub fn protocol_state_view(state: &MinaStateProtocolStateValueStableV2) -> ProtocolStateView {
        let cs = &state.body.consensus_state;
        let sed = &cs.staking_epoch_data;
        let ned = &cs.staking_epoch_data;

        ProtocolStateView {
            snarked_ledger_hash: state.body.blockchain_state.registers.ledger.to_field(),
            timestamp: BlockTime(state.body.blockchain_state.timestamp.as_u64()),
            blockchain_length: Length(cs.blockchain_length.as_u32()),
            min_window_density: Length(cs.min_window_density.as_u32()),
            last_vrf_output: (),
            total_currency: Amount(cs.total_currency.as_u64()),
            global_slot_since_hard_fork: Slot(cs.curr_global_slot.slot_number.as_u32()), // TODO: Check if it's correct
            global_slot_since_genesis: Slot(cs.global_slot_since_genesis.as_u32()),
            staking_epoch_data: EpochData {
                ledger: EpochLedger {
                    hash: sed.ledger.hash.to_field(),
                    total_currency: Amount(sed.ledger.total_currency.as_u64()),
                },
                seed: sed.seed.to_field(),
                start_checkpoint: sed.start_checkpoint.to_field(),
                lock_checkpoint: sed.lock_checkpoint.to_field(),
                epoch_length: Length(sed.epoch_length.as_u32()),
            },
            next_epoch_data: EpochData {
                ledger: EpochLedger {
                    hash: ned.ledger.hash.to_field(),
                    total_currency: Amount(ned.ledger.total_currency.as_u64()),
                },
                seed: ned.seed.to_field(),
                start_checkpoint: ned.start_checkpoint.to_field(),
                lock_checkpoint: ned.lock_checkpoint.to_field(),
                epoch_length: Length(ned.epoch_length.as_u32()),
            },
        }
    }
}

pub mod local_state {
    use ark_ff::Zero;

    use crate::{hash_with_kimchi, scan_state::currency::Signed, Inputs};

    use super::{zkapp_command::CallForest, *};

    pub struct StackFrame {
        caller: TokenId,
        caller_caller: TokenId,
        calls: CallForest, // TODO
    }

    impl StackFrame {
        pub fn empty() -> Self {
            Self {
                caller: TokenId::default(),
                caller_caller: TokenId::default(),
                calls: CallForest(Vec::new()),
            }
        }

        /// TODO: this needs to be tested
        ///
        /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/stack_frame.ml#L90
        pub fn hash(&self) -> Fp {
            let mut inputs = Inputs::new();

            inputs.append_field(self.caller.0);
            inputs.append_field(self.caller_caller.0);

            let field = match self.calls.0.get(0) {
                None => Fp::zero(),
                Some(call) => call.stack_hash,
            };
            inputs.append_field(field);

            hash_with_kimchi("MinaAcctUpdStckFrm", &inputs.to_fields())
        }
    }

    pub struct LocalState {
        stack_frame: Fp,
        call_stack: Fp,
        transaction_commitment: Fp,
        full_transaction_commitment: Fp,
        token_id: TokenId,
        excess: Signed<Amount>,
        supply_increase: Signed<Amount>,
        ledger: Fp,
        success: bool,
        account_update_index: Index,
        failure_status_tbl: Vec<Vec<TransactionFailure>>,
    }

    impl LocalState {
        /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_state/local_state.ml#L63
        pub fn dummy() -> Self {
            Self {
                stack_frame: StackFrame::empty().hash(),
                call_stack: Fp::zero(),
                transaction_commitment: Fp::zero(),
                full_transaction_commitment: Fp::zero(),
                token_id: TokenId::default(),
                excess: Signed::<Amount>::zero(),
                supply_increase: Signed::<Amount>::zero(),
                ledger: Fp::zero(),
                success: true,
                account_update_index: Index(0),
                failure_status_tbl: Vec::new(),
            }
        }

        pub fn empty() -> Self {
            Self::dummy()
        }
    }
}

pub fn apply_transaction<L>(
    constraint_constants: &ConstraintConstants,
    txn_state_view: &ProtocolStateView,
    mut ledger: L,
    transaction: Transaction,
) -> Result<TransactionApplied, String>
where
    L: BaseLedger,
{
    use Transaction::*;
    use UserCommand::*;

    let previous_hash = ledger.merkle_root();
    let txn_global_slot = &txn_state_view.global_slot_since_genesis;

    match transaction {
        Command(SignedCommand(cmd)) => todo!(),
        Command(ZkAppCommand(cmd)) => todo!(),
        FeeTransfer(fee_transfer) => apply_fee_transfer(
            constraint_constants,
            txn_global_slot,
            &mut ledger,
            fee_transfer,
        )
        .map(Varying::FeeTransfer),
        Coinbase(_) => todo!(),
    }
    .map(|varying| TransactionApplied {
        previous_hash,
        varying,
    })
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L1991
fn apply_fee_transfer<L>(
    constraint_constants: &ConstraintConstants,
    txn_global_slot: &Slot,
    ledger: &mut L,
    fee_transfer: FeeTransfer,
) -> Result<transaction_applied::FeeTransferApplied, String>
where
    L: BaseLedger,
{
    let (new_accounts, failures, burned_tokens) = process_fee_transfer(
        ledger,
        &fee_transfer,
        |action, _, balance, fee| {
            let amount = {
                let amount = Amount::of_fee(fee);
                sub_account_creation_fee(constraint_constants, action, amount)?
            };
            add_amount(balance, amount)
        },
        |account| update_timing_when_no_deduction(txn_global_slot, account),
    )?;

    let status = if failures.iter().all(Vec::is_empty) {
        TransactionStatus::Applied
    } else {
        TransactionStatus::Failed(failures)
    };

    Ok(transaction_applied::FeeTransferApplied {
        fee_transfer: WithStatus {
            data: fee_transfer,
            status,
        },
        new_accounts,
        burned_tokens,
    })
}

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L607
fn sub_account_creation_fee(
    constraint_constants: &ConstraintConstants,
    action: AccountState,
    amount: Amount,
) -> Result<Amount, String> {
    let fee = &constraint_constants.account_creation_fee;

    match action {
        AccountState::Added => {
            if let Some(amount) = amount.checked_sub(&Amount::of_fee(fee)) {
                return Ok(amount);
            }
            Err(format!(
                "Error subtracting account creation fee {:?}; transaction amount {:?} insufficient",
                fee, amount
            ))
        }
        AccountState::Existed => Ok(amount),
    }
}

fn update_timing_when_no_deduction(
    txn_global_slot: &Slot,
    account: &Account,
) -> Result<Timing, String> {
    validate_timing(account, Amount::zero(), txn_global_slot)
}

/// TODO: Move this to the ledger
/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_ledger/ledger.ml#L311
fn get_or_create<L>(
    ledger: &mut L,
    account_id: &AccountId,
) -> Result<(AccountState, Account, Address), String>
where
    L: BaseLedger,
{
    let location = ledger
        .get_or_create_account(account_id.clone(), Account::initialize(account_id))
        .map_err(|e| format!("{:?}", e))?;

    let action = match location {
        GetOrCreated::Added(_) => AccountState::Added,
        GetOrCreated::Existed(_) => AccountState::Existed,
    };

    let addr = location.addr();

    let account = ledger
        .get(addr.clone())
        .expect("get_or_create: Account was not found in the ledger after creation");

    Ok((action, account, addr))
}

fn get_new_accounts<T>(action: AccountState, data: T) -> Option<T> {
    match action {
        AccountState::Added => Some(data),
        AccountState::Existed => None,
    }
}

/// Structure of the failure status:
///  I. Only one fee transfer in the transaction (`One) and it fails:
///     [[failure]]
///  II. Two fee transfers in the transaction (`Two)-
///   Both fee transfers fail:
///     [[failure-of-first-fee-transfer]; [failure-of-second-fee-transfer]]
///   First succeeds and second one fails:
///     [[];[failure-of-second-fee-transfer]]
///   First fails and second succeeds:
///     [[failure-of-first-fee-transfer];[]]
fn process_fee_transfer<L, FunBalance, FunTiming>(
    ledger: &mut L,
    fee_transfer: &FeeTransfer,
    modify_balance: FunBalance,
    modify_timing: FunTiming,
) -> Result<(Vec<AccountId>, Vec<Vec<TransactionFailure>>, Amount), String>
where
    L: BaseLedger,
    FunTiming: Fn(&Account) -> Result<Timing, String>,
    FunBalance: Fn(AccountState, &AccountId, Balance, &Fee) -> Result<Balance, String>,
{
    if !fee_transfer.fee_tokens().all(TokenId::is_default) {
        return Err("Cannot pay fees in non-default tokens.".to_string());
    }

    match &**fee_transfer {
        OneOrTwo::One(fee_transfer) => {
            let account_id = fee_transfer.receiver();
            let (a, action, can_receive) = has_permission_to_receive(ledger, &account_id);

            let timing = modify_timing(&a)?;
            let balance =
                modify_balance(action, &account_id, Balance(a.balance), &fee_transfer.fee)?;

            if can_receive.0 {
                let (_, mut account, loc) = get_or_create(ledger, &account_id)?;
                let new_accounts = get_new_accounts(action, account_id.clone());

                account.balance = balance.0;
                account.timing = timing;

                ledger.set(loc, account);

                let new_accounts: Vec<_> = [new_accounts].into_iter().flatten().collect();
                Ok((new_accounts, vec![], Amount::zero()))
            } else {
                Ok((vec![], single_failure(), Amount::of_fee(&fee_transfer.fee)))
            }
        }
        OneOrTwo::Two((fee_transfer1, fee_transfer2)) => {
            let account_id1 = fee_transfer1.receiver();
            let (a1, action1, can_receive1) = has_permission_to_receive(ledger, &account_id1);

            let account_id2 = fee_transfer2.receiver();

            if account_id1 == account_id2 {
                let fee = fee_transfer1
                    .fee
                    .checked_add(&fee_transfer2.fee)
                    .ok_or_else(|| "Overflow".to_string())?;

                let timing = modify_timing(&a1)?;
                let balance = modify_balance(action1, &account_id1, Balance(a1.balance), &fee)?;

                if can_receive1.0 {
                    let (_, mut a1, l1) = get_or_create(ledger, &account_id1)?;
                    let new_accounts1 = get_new_accounts(action1, account_id1);

                    a1.balance = balance.0;
                    a1.timing = timing;

                    ledger.set(l1, a1);

                    let new_accounts: Vec<_> = [new_accounts1].into_iter().flatten().collect();
                    Ok((new_accounts, vec![vec![], vec![]], Amount::zero()))
                } else {
                    // failure for each fee transfer single

                    Ok((
                        vec![],
                        vec![
                            vec![TransactionFailure::Update_not_permitted_balance],
                            vec![TransactionFailure::Update_not_permitted_balance],
                        ],
                        Amount::of_fee(&fee),
                    ))
                }
            } else {
                let (a2, action2, can_receive2) = has_permission_to_receive(ledger, &account_id2);

                let balance1 = modify_balance(
                    action1,
                    &account_id1,
                    Balance(a1.balance),
                    &fee_transfer1.fee,
                )?;

                // Note: Not updating the timing field of a1 to avoid additional check
                // in transactions snark (check_timing for "receiver"). This is OK
                // because timing rules will not be violated when balance increases
                // and will be checked whenever an amount is deducted from the account. (#5973)*)

                let timing2 = modify_timing(&a2)?;
                let balance2 = modify_balance(
                    action2,
                    &account_id2,
                    Balance(a2.balance),
                    &fee_transfer2.fee,
                )?;

                let (new_accounts1, failures1, burned_tokens1) = if can_receive1.0 {
                    let (_, mut a1, l1) = get_or_create(ledger, &account_id1)?;
                    let new_accounts1 = get_new_accounts(action1, account_id1);

                    a1.balance = balance1.0;
                    ledger.set(l1, a1);

                    (new_accounts1, vec![], Amount::zero())
                } else {
                    (
                        None,
                        vec![TransactionFailure::Update_not_permitted_balance],
                        Amount::of_fee(&fee_transfer1.fee),
                    )
                };

                let (new_accounts2, failures2, burned_tokens2) = if can_receive2.0 {
                    let (_, mut a2, l2) = get_or_create(ledger, &account_id2)?;
                    let new_accounts2 = get_new_accounts(action2, account_id2);

                    a2.balance = balance2.0;
                    a2.timing = timing2;

                    ledger.set(l2, a2);

                    (new_accounts2, vec![], Amount::zero())
                } else {
                    (
                        None,
                        vec![TransactionFailure::Update_not_permitted_balance],
                        Amount::of_fee(&fee_transfer2.fee),
                    )
                };

                let burned_tokens = burned_tokens1
                    .checked_add(&burned_tokens2)
                    .ok_or_else(|| "burned tokens overflow".to_string())?;

                let new_accounts: Vec<_> = [new_accounts1, new_accounts2]
                    .into_iter()
                    .flatten()
                    .collect();
                let failures: Vec<_> = [failures1, failures2].into_iter().collect();

                Ok((new_accounts, failures, burned_tokens))
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum AccountState {
    Added,
    Existed,
}

#[derive(Debug)]
struct HasPermissionToReceive(bool);

/// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/transaction_logic/mina_transaction_logic.ml#L1852
fn has_permission_to_receive<L>(
    ledger: &mut L,
    receiver_account_id: &AccountId,
) -> (Account, AccountState, HasPermissionToReceive)
where
    L: BaseLedger,
{
    use crate::PermissionTo::*;
    use AccountState::*;

    let init_account = Account::initialize(receiver_account_id);

    match ledger.location_of_account(&receiver_account_id) {
        None => {
            // new account, check that default permissions allow receiving
            let perm = init_account.has_permission_to(Receive);
            (init_account, Added, HasPermissionToReceive(perm))
        }
        Some(location) => match ledger.get(location) {
            None => panic!("Ledger location with no account"),
            Some(receiver_account) => {
                let perm = receiver_account.has_permission_to(Receive);
                (receiver_account, Existed, HasPermissionToReceive(perm))
            }
        },
    }
}

fn validate_time(valid_until: &Slot, current_global_slot: &Slot) -> Result<(), String> {
    if current_global_slot <= valid_until {
        return Ok(());
    }

    Err(format!(
        "Current global slot {:?} greater than transaction expiry slot {:?}",
        current_global_slot, valid_until
    ))
}

pub fn apply_user_command_unchecked<L>(
    constraint_constants: &ConstraintConstants,
    txn_state_view: &ProtocolStateView,
    txn_global_slot: &Slot,
    ledger: &mut L,
    user_command: SignedCommand,
) -> Result<(), String>
where
    L: BaseLedger,
{
    let SignedCommand {
        payload,
        signer: signer_pk,
        signature,
    } = &user_command;
    let current_global_slot = txn_global_slot;

    let valid_until = user_command.valid_until();
    validate_time(&valid_until, current_global_slot)?;

    // Fee-payer information
    let fee_payer = user_command.fee_payer();
    let (fee_payer_location, fee_payer_account) =
        pay_fee(&user_command, signer_pk, ledger, current_global_slot)?;

    // TODO: The rest is implemented on the branch `transaction_fuzzer`

    Ok(())
}

fn pay_fee<L>(
    user_command: &SignedCommand,
    signer_pk: &CompressedPubKey,
    ledger: &mut L,
    current_global_slot: &Slot,
) -> Result<(ExistingOrNew, Account), String>
where
    L: BaseLedger,
{
    let nonce = user_command.nonce();
    let fee_payer = user_command.fee_payer();
    let fee_token = user_command.fee_token();

    if &fee_payer.public_key != signer_pk {
        return Err("Cannot pay fees from a public key that did not sign the transaction".into());
    }

    if fee_token != TokenId::default() {
        return Err("Cannot create transactions with fee_token different from the default".into());
    }

    pay_fee_impl(
        &user_command.payload,
        nonce,
        fee_payer,
        user_command.fee(),
        ledger,
        current_global_slot,
    )
}

fn pay_fee_impl<L>(
    command: &SignedCommandPayload,
    nonce: AccountNonce,
    fee_payer: AccountId,
    fee: Fee,
    ledger: &mut L,
    current_global_slot: &Slot,
) -> Result<(ExistingOrNew, Account), String>
where
    L: BaseLedger,
{
    // Fee-payer information
    let (location, mut account) = get_with_location(ledger, &fee_payer)?;

    if let ExistingOrNew::New = location {
        return Err("The fee-payer account does not exist".to_string());
    };

    let fee = Amount::of_fee(&fee);
    let balance = sub_amount(Balance(account.balance), fee.clone())?;

    validate_nonces(nonce, AccountNonce(account.nonce))?;
    let timing = validate_timing(&account, fee, current_global_slot)?;

    account.balance = balance.as_u64();
    account.nonce = account.nonce.wrapping_add(1); // TODO: Not sure if OCaml wraps
    account.receipt_chain_hash = cons_signed_command_payload(command, account.receipt_chain_hash);
    account.timing = timing;

    Ok((location, account))

    // in
    // ( location
    // , { account with
    //     balance
    //   ; nonce = Account.Nonce.succ account.nonce
    //   ; receipt_chain_hash =
    //       Receipt.Chain_hash.cons_signed_command_payload command
    //         account.receipt_chain_hash
    //   ; timing
    //   } )
}

pub mod transaction_union_payload {
    use mina_hasher::ROInput as LegacyInput;

    use crate::scan_state::transaction_logic::signed_command::{
        PaymentPayload, StakeDelegationPayload,
    };

    use super::*;

    struct Common {
        fee: Fee,
        fee_token: TokenId,
        fee_payer_pk: CompressedPubKey,
        nonce: AccountNonce,
        valid_until: Slot,
        memo: Memo,
    }

    #[derive(Clone, Debug)]
    enum Tag {
        Payment = 0,
        StakeDelegation = 1,
        CreateAccount = 2,
        MintTokens = 3,
        FeeTransfer = 4,
        Coinbase = 5,
    }

    struct Body {
        tag: Tag,
        source_pk: CompressedPubKey,
        receiver_pk: CompressedPubKey,
        token_id: TokenId,
        amount: Amount,
        token_locked: bool,
    }

    pub struct TransactionUnionPayload {
        common: Common,
        body: Body,
    }

    impl TransactionUnionPayload {
        pub fn of_user_command_payload(payload: &SignedCommandPayload) -> Self {
            use signed_command::Body::{Payment, StakeDelegation};

            Self {
                common: Common {
                    fee: payload.common.fee.clone(),
                    fee_token: TokenId::default(),
                    fee_payer_pk: payload.common.fee_payer_pk.clone(),
                    nonce: payload.common.nonce.clone(),
                    valid_until: payload.common.valid_until.clone(),
                    memo: payload.common.memo.clone(),
                },
                body: match &payload.body {
                    Payment(PaymentPayload {
                        source_pk,
                        receiver_pk,
                        amount,
                    }) => Body {
                        tag: Tag::Payment,
                        source_pk: source_pk.clone(),
                        receiver_pk: receiver_pk.clone(),
                        token_id: TokenId::default(),
                        amount: amount.clone(),
                        token_locked: false,
                    },
                    StakeDelegation(StakeDelegationPayload::SetDelegate {
                        delegator,
                        new_delegate,
                    }) => Body {
                        tag: Tag::StakeDelegation,
                        source_pk: delegator.clone(),
                        receiver_pk: new_delegate.clone(),
                        token_id: TokenId::default(),
                        amount: Amount::zero(),
                        token_locked: false,
                    },
                },
            }
        }

        /// TODO: Needs to be tested, the order might be reversed
        ///
        /// https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/transaction_union_payload.ml#L309
        pub fn to_input_legacy(&self) -> LegacyInput {
            let mut roi = LegacyInput::new();

            // Self.common
            {
                roi = roi.append_u64(self.common.fee.0);

                // TokenId.default
                // https://github.com/MinaProtocol/mina/blob/2ee6e004ba8c6a0541056076aab22ea162f7eb3a/src/lib/mina_base/signed_command_payload.ml#L19
                roi = roi.append_bool(true);
                for _ in 0..63 {
                    roi = roi.append_bool(false);
                }

                // fee_payer_pk
                roi = roi.append_field(self.common.fee_payer_pk.x);
                roi = roi.append_bool(self.common.fee_payer_pk.is_odd);

                // nonce
                roi = roi.append_u32(self.common.nonce.0);

                // valid_until
                roi = roi.append_u32(self.common.valid_until.0);

                // memo
                roi = roi.append_bytes(&self.common.memo);
            }

            // Self.body
            {
                // tag
                let tag = self.body.tag.clone() as u8;
                for bit in [4, 2, 1] {
                    roi = roi.append_bool(tag & bit != 0);
                }

                // source_pk
                roi = roi.append_field(self.body.source_pk.x);
                roi = roi.append_bool(self.body.source_pk.is_odd);

                // receiver_pk
                roi = roi.append_field(self.body.receiver_pk.x);
                roi = roi.append_bool(self.body.receiver_pk.is_odd);

                // amount
                roi = roi.append_u64(self.body.amount.0);

                // token_locked
                roi = roi.append_bool(self.body.token_locked);
            }

            roi
        }
    }
}

/// Returns the new `receipt_chain_hash`
fn cons_signed_command_payload(
    command_payload: &SignedCommandPayload,
    last_receipt_chain_hash: ReceiptChainHash,
) -> ReceiptChainHash {
    // Note: Not sure why the use the legacy way of hashing here

    use mina_hasher::ROInput as LegacyInput;

    let union = TransactionUnionPayload::of_user_command_payload(command_payload);

    let inputs = union.to_input_legacy();
    let inputs = inputs.append_field(last_receipt_chain_hash.0);

    use mina_hasher::{create_legacy, Hashable, Hasher, ROInput};

    #[derive(Clone)]
    struct MyInput(LegacyInput);

    impl Hashable for MyInput {
        type D = ();

        fn to_roinput(&self) -> ROInput {
            self.0.clone()
        }

        fn domain_string(_: Self::D) -> Option<String> {
            Some("MinaReceiptUC".to_string())
        }
    }

    let mut hasher = create_legacy::<MyInput>(());
    hasher.update(&MyInput(inputs));
    ReceiptChainHash(hasher.digest())
}

fn validate_nonces(txn_nonce: AccountNonce, account_nonce: AccountNonce) -> Result<(), String> {
    if account_nonce == txn_nonce {
        return Ok(());
    }

    Err(format!(
        "Nonce in account {:?} different from nonce in transaction {:?}",
        account_nonce, txn_nonce,
    ))
}

fn validate_timing(
    account: &Account,
    txn_amount: Amount,
    txn_global_slot: &Slot,
) -> Result<Timing, String> {
    let (timing, _) = validate_timing_with_min_balance(account, txn_amount, txn_global_slot)?;

    Ok(timing)
}

fn validate_timing_with_min_balance(
    account: &Account,
    txn_amount: Amount,
    txn_global_slot: &Slot,
) -> Result<(Timing, MinBalance), String> {
    use TimingValidation::*;

    let (possibly_error, timing, min_balance) =
        validate_timing_with_min_balance_impl(account, txn_amount.clone(), txn_global_slot);

    match possibly_error {
        InsufficientBalance(true) => Err(format!(
            "For timed account, the requested transaction for amount {:?} \
             at global slot {:?}, the balance {:?} \
             is insufficient",
            txn_amount, txn_global_slot, account.balance
        )),
        InvalidTiming(true) => Err(format!(
            "For timed account, the requested transaction for amount {:?} \
             at global slot {:?}, applying the transaction would put the \
             balance below the calculated minimum balance of {:?}",
            txn_amount, txn_global_slot, min_balance.0
        )),
        InsufficientBalance(false) => {
            panic!("Broken invariant in validate_timing_with_min_balance'")
        }
        InvalidTiming(false) => Ok((timing, min_balance)),
    }
}

enum TimingValidation {
    InsufficientBalance(bool),
    InvalidTiming(bool),
}

#[derive(Debug)]
struct MinBalance(Balance);

fn validate_timing_with_min_balance_impl(
    account: &Account,
    txn_amount: Amount,
    txn_global_slot: &Slot,
) -> (TimingValidation, Timing, MinBalance) {
    use crate::Timing::*;
    use TimingValidation::*;

    match account.timing {
        Untimed => {
            // no time restrictions
            match Balance(account.balance).sub_amount(txn_amount) {
                None => (
                    InsufficientBalance(true),
                    Untimed,
                    MinBalance(Balance::zero()),
                ),
                Some(_) => (
                    InsufficientBalance(false),
                    Untimed,
                    MinBalance(Balance::zero()),
                ),
            }
        }
        Timed {
            initial_minimum_balance,
            cliff_time,
            cliff_amount,
            vesting_period,
            vesting_increment,
        } => {
            let account_balance = Balance(account.balance);
            let initial_minimum_balance = Balance(initial_minimum_balance);

            let (invalid_balance, invalid_timing, curr_min_balance) =
                match account_balance.sub_amount(txn_amount) {
                    None => {
                        // NB: The [initial_minimum_balance] here is the incorrect value,
                        // but:
                        // * we don't use it anywhere in this error case; and
                        // * we don't want to waste time computing it if it will be unused.
                        (true, false, initial_minimum_balance)
                    }
                    Some(proposed_new_balance) => {
                        let cliff_time = Slot(cliff_time);
                        let cliff_amount = Amount(cliff_amount);
                        let vesting_period = Slot(vesting_period);
                        let vesting_increment = Amount(vesting_increment);

                        let curr_min_balance = account_min_balance_at_slot(
                            txn_global_slot.clone(),
                            cliff_time,
                            cliff_amount,
                            vesting_period,
                            vesting_increment,
                            initial_minimum_balance,
                        );

                        if proposed_new_balance < curr_min_balance {
                            (false, true, curr_min_balance)
                        } else {
                            (false, false, curr_min_balance)
                        }
                    }
                };

            // once the calculated minimum balance becomes zero, the account becomes untimed
            let possibly_error = if invalid_balance {
                InsufficientBalance(invalid_balance)
            } else {
                InvalidTiming(invalid_timing)
            };

            if curr_min_balance > Balance::zero() {
                (
                    possibly_error,
                    account.timing.clone(),
                    MinBalance(curr_min_balance),
                )
            } else {
                (possibly_error, Untimed, MinBalance(Balance::zero()))
            }
        }
    }
}

// TODO: This should be in `account.rs`
fn account_min_balance_at_slot(
    global_slot: Slot,
    cliff_time: Slot,
    cliff_amount: Amount,
    vesting_period: Slot,
    vesting_increment: Amount,
    initial_minimum_balance: Balance,
) -> Balance {
    if global_slot < cliff_time {
        initial_minimum_balance
    } else if vesting_period.is_zero() {
        // If vesting period is zero then everything vests immediately at the cliff
        Balance::zero()
    } else {
        match initial_minimum_balance.sub_amount(cliff_amount) {
            None => Balance::zero(),
            Some(min_balance_past_cliff) => {
                // take advantage of fact that global slots are uint32's

                let num_periods =
                    (global_slot.as_u32() - cliff_time.as_u32()) / vesting_period.as_u32();
                let num_periods: u64 = num_periods.try_into().unwrap();

                let vesting_decrement = {
                    let vesting_increment = vesting_increment.as_u64();

                    if u64::MAX
                        .checked_div(num_periods)
                        .map(|res| matches!(res.cmp(&vesting_increment), std::cmp::Ordering::Less))
                        .unwrap_or(false)
                    {
                        // The vesting decrement will overflow, use [max_int] instead.
                        Amount::from_u64(u64::MAX)
                    } else {
                        Amount::from_u64(num_periods.checked_mul(vesting_increment).unwrap())
                    }
                };

                match min_balance_past_cliff.sub_amount(vesting_decrement) {
                    None => Balance::zero(),
                    Some(amount) => amount,
                }
            }
        }
    }
}

fn sub_amount(balance: Balance, amount: Amount) -> Result<Balance, String> {
    balance
        .sub_amount(amount)
        .ok_or_else(|| "insufficient funds".to_string())
}

fn add_amount(balance: Balance, amount: Amount) -> Result<Balance, String> {
    balance
        .add_amount(amount)
        .ok_or_else(|| "overflow".to_string())
}

pub enum ExistingOrNew {
    Existing(Address),
    New,
}

fn get_with_location<L>(
    ledger: &mut L,
    account_id: &AccountId,
) -> Result<(ExistingOrNew, Account), String>
where
    L: BaseLedger,
{
    match ledger.location_of_account(account_id) {
        Some(location) => match ledger.get(location.clone()) {
            Some(account) => Ok((ExistingOrNew::Existing(location), account)),
            None => panic!("Ledger location with no account"),
        },
        None => Ok((
            ExistingOrNew::New,
            Account::create_with(account_id.clone(), 0),
        )),
    }
}
