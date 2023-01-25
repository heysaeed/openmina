use ark_ff::Zero;
use mina_hasher::Fp;
use mina_signer::CompressedPubKey;

use crate::{
    check_permission, hash_with_kimchi,
    scan_state::{
        currency::{Amount, Balance, Fee, Index, Magnitude, Sgn, Signed, Slot},
        scan_state::ConstraintConstants,
        transaction_logic::{
            account_check_timing, get_account, is_timed,
            local_state::{CallStack, LocalStateEnv, StackFrame},
            protocol_state::GlobalState,
            set_account,
            zkapp_command::{
                self, AccountUpdate, CallForest, CheckAuthorizationResult, OrIgnore, ZkAppCommand,
            },
            Env, TimingValidation, TransactionFailure,
        },
    },
    staged_ledger::sparse_ledger::LedgerIntf,
    Account, AuthRequired, ControlTag, Inputs, Mask, ReceiptChainHash, Timing, TokenId,
    ZkAppAccount,
};

use super::transaction_logic::{zkapp_command::SequenceEvents, Eff, ExistingOrNew, PerformResult};

pub struct StartData {
    pub zkapp_command: ZkAppCommand,
    pub memo_hash: Fp,
}

pub enum IsStart {
    Yes(StartData),
    No,
    Compute(StartData),
}

pub struct Handler<L: LedgerIntf + Clone> {
    pub perform: fn(Eff<L>) -> PerformResult<L>,
}

pub fn commitment(account_updates: CallForest<AccountUpdate>) -> ReceiptChainHash {
    ReceiptChainHash(account_updates.hash())
}

pub fn full_commitment(
    account_update: AccountUpdate,
    memo_hash: Fp,
    commitment: ReceiptChainHash,
) -> ReceiptChainHash {
    let fee_payer_hash = account_update.digest();
    ReceiptChainHash(hash_with_kimchi(
        "MinaAcctUpdateCons",
        &[memo_hash, fee_payer_hash, commitment.0],
    ))
}

pub struct TimingAsRecord {
    is_timed: bool,
    initial_minimum_balance: Balance,
    cliff_time: Slot,
    cliff_amount: Amount,
    vesting_period: Slot,
    vesting_increment: Amount,
}

pub fn to_record(t: Timing) -> TimingAsRecord {
    match t {
        Timing::Untimed => TimingAsRecord {
            is_timed: false,
            initial_minimum_balance: Balance::zero(),
            cliff_time: Slot::zero(),
            cliff_amount: Amount::zero(),
            vesting_period: Slot::from_u32(1),
            vesting_increment: Amount::zero(),
        },
        Timing::Timed {
            initial_minimum_balance,
            cliff_time,
            cliff_amount,
            vesting_period,
            vesting_increment,
        } => TimingAsRecord {
            is_timed: true,
            initial_minimum_balance,
            cliff_time,
            cliff_amount,
            vesting_period,
            vesting_increment,
        },
    }
}

pub fn controller_check(
    proof_verifies: bool,
    signature_verifies: bool,
    perm: AuthRequired,
) -> bool {
    assert!(!(proof_verifies && signature_verifies));
    let tag = if proof_verifies {
        ControlTag::Proof
    } else if signature_verifies {
        ControlTag::Signature
    } else {
        ControlTag::NoneGiven
    };
    check_permission(perm, tag)
}

pub enum ZkAppCommandElt {
    ZkAppCommandCommitment(ReceiptChainHash),
}

pub fn cons_zkapp_command_commitment(index: u32, e: ZkAppCommandElt, t: ReceiptChainHash) -> Fp {
    let mut inputs = Inputs::new();

    let ZkAppCommandElt::ZkAppCommandCommitment(receipt) = &e;

    inputs.append(&t);
    inputs.append(receipt);
    inputs.append_u32(index);

    hash_with_kimchi("MinaReceiptUC", &inputs.to_fields())
}

fn assert_with_failure_status_tbl(b: bool, failure_status_tbl: Vec<Vec<TransactionFailure>>) {
    if !b && !(failure_status_tbl.is_empty()) {
        panic!("{:?}", failure_status_tbl);
    } else {
        assert!(b)
    }
}

pub fn set_ledger<L>(should_update: bool, t: GlobalState<L>, ledger: L) -> GlobalState<L>
where
    L: LedgerIntf + Clone,
{
    if should_update {
        t.ledger.apply_mask(ledger);
    }
    t
}

// https://github.com/MinaProtocol/mina/blob/32a91613c388a71f875581ad72276e762242f802/src/lib/mina_ledger/ledger.ml#L211
fn empty_ledger(depth: usize) -> Mask {
    let mask = Mask::new_unattached(depth);
    //mask.set_parent(parent, None)
    mask
}

fn pop_call_stack(s: &CallStack) -> (StackFrame, CallStack) {
    if let Some(a) = s.pop() {
        a
    } else {
        (StackFrame::default(), CallStack::new())
    }
}

pub struct GetNextAccountUpdateResult {
    pub account_update: AccountUpdate,
    pub account_update_forest: CallForest<AccountUpdate>,
    pub new_call_stack: CallStack,
    pub new_frame: StackFrame,
}

pub fn get_next_account_update(
    current_forest: StackFrame,
    call_stack: CallStack,
) -> GetNextAccountUpdateResult {
    let (next_forest, next_call_stack) = pop_call_stack(&call_stack);
    let (current_forest, call_stack) = if let true = current_forest.calls.is_empty() {
        (next_forest, next_call_stack)
    } else {
        (current_forest, call_stack)
    };

    let ((account_update, account_update_forest), remainder_of_current_forest) =
        current_forest.calls.pop_exn();
    let account_update_caller = account_update.caller();
    let is_normal_call = account_update_caller == current_forest.caller;
    let is_delegate_call = account_update_caller == current_forest.caller_caller;

    assert!(is_normal_call || is_delegate_call);

    let account_update_forest_empty = account_update_forest.is_empty();
    let remainder_of_current_forest_empty = remainder_of_current_forest.is_empty();
    let (newly_popped_frame, popped_call_stack) = pop_call_stack(&call_stack);
    let remainder_of_current_forest_frame = StackFrame {
        caller: current_forest.caller.clone(),
        caller_caller: current_forest.caller_caller,
        calls: remainder_of_current_forest,
    };

    let new_call_stack = if let true = account_update_forest_empty {
        if let true = remainder_of_current_forest_empty {
            popped_call_stack
        } else {
            call_stack
        }
    } else {
        if let true = remainder_of_current_forest_empty {
            call_stack
        } else {
            call_stack.push(&remainder_of_current_forest_frame)
        }
    };

    let new_frame = if let true = account_update_forest_empty {
        if let true = remainder_of_current_forest_empty {
            newly_popped_frame
        } else {
            remainder_of_current_forest_frame
        }
    } else {
        let caller = if let true = is_normal_call {
            account_update.account_id().derive_token_id()
        } else {
            current_forest.caller.clone()
        };
        StackFrame {
            caller,
            caller_caller: account_update_caller,
            calls: account_update_forest.clone(),
        }
    };
    GetNextAccountUpdateResult {
        account_update,
        account_update_forest,
        new_frame,
        new_call_stack,
    }
}

pub fn check_account<L>(
    public_key: CompressedPubKey,
    token_id: TokenId,
    (account, loc): (&Account, &ExistingOrNew<L::Location>),
) -> bool
where
    L: LedgerIntf,
{
    assert!(public_key == account.public_key);
    assert!(token_id == account.token_id);
    // IsNew?
    match loc {
        ExistingOrNew::Existing(_) => false,
        ExistingOrNew::New => true,
    }
}

pub fn make_zkapp(a: Account) -> Account {
    let zkapp = if let zkapp @ Some(_) = a.zkapp {
        zkapp
    } else {
        Some(ZkAppAccount::default())
    };
    Account { zkapp, ..a }
}

pub fn update_sequence_state(
    sequence_state: [Fp; 5],
    sequence_events: SequenceEvents,
    txn_global_slot: Slot,
    last_sequence_slot: Slot,
) -> ([Fp; 5], Slot) {
    let [_s1, _s2, _s3, _s4, _s5] = sequence_state;
    let is_empty = sequence_events.is_empty();
    let s1_updated = sequence_events.push_events(_s1);
    let s1 = if let true = is_empty { _s1 } else { s1_updated };
    let is_this_slot = txn_global_slot == last_sequence_slot;
    let is_empty_or_this_slot = is_empty || is_this_slot;
    let (s5, s4, s3, s2) = if let true = is_empty_or_this_slot {
        (_s5, _s4, _s3, _s2)
    } else {
        (_s4, _s3, _s2, _s1)
    };
    let last_sequence_slot = if let true = is_empty {
        last_sequence_slot
    } else {
        txn_global_slot
    };
    ([s1, s2, s3, s4, s5], last_sequence_slot)
}

pub fn unmake_zkapp(a: Account) -> Account {
    let zkapp = if let Some(zkapp) = a.zkapp {
        if let true = ZkAppAccount::default() == zkapp {
            None
        } else {
            Some(zkapp)
        }
    } else {
        None
    };
    Account { zkapp, ..a }
}

pub fn apply<L>(
    constraint_constants: &ConstraintConstants,
    is_start: IsStart,
    _h: Handler<L>,
    (global_state, local_state): (GlobalState<L>, LocalStateEnv<L>),
) -> (GlobalState<L>, LocalStateEnv<L>)
where
    L: LedgerIntf + Clone,
{
    let is_start_ = local_state.stack_frame.calls.is_empty();

    match is_start {
        IsStart::No => (),
        IsStart::Yes(_) => assert!(is_start_),
        IsStart::Compute(_) => assert_ne!(is_start_, true),
    };

    let is_start_ = match is_start {
        IsStart::Yes(_) => true,
        IsStart::No => false,
        IsStart::Compute(_) => is_start_,
    };

    let mut local_state = local_state.clone();

    if is_start_ {
        local_state.ledger = global_state.ledger.clone();
    }

    let (
        (account_update, remaining, call_stack),
        account_update_forest,
        local_state,
        (a, inclusion_proof),
    ) = {
        let (to_pop, call_stack) = match &is_start {
            IsStart::Compute(start_data) => {
                if let true = is_start_ {
                    (
                        StackFrame {
                            caller: TokenId::default(),
                            caller_caller: TokenId::default(),
                            calls: start_data.zkapp_command.account_updates.clone(),
                        },
                        CallStack::new(),
                    )
                } else {
                    (
                        local_state.stack_frame.clone(),
                        local_state.call_stack.clone(),
                    )
                }
            }
            IsStart::Yes(start_data) => (
                StackFrame {
                    caller: TokenId::default(),
                    caller_caller: TokenId::default(),
                    calls: start_data.zkapp_command.account_updates.clone(),
                },
                CallStack::new(),
            ),
            IsStart::No => (
                local_state.stack_frame.clone(),
                local_state.call_stack.clone(),
            ),
        };

        let GetNextAccountUpdateResult {
            account_update,
            account_update_forest,
            new_frame,
            new_call_stack: _,
        } = get_next_account_update(to_pop, call_stack.clone());
        let remaining = new_frame;

        let mut local_state = local_state.add_check(
            TransactionFailure::TokenOwnerNotCaller,
            account_update.token_id() == TokenId::default()
                || account_update.token_id() == account_update.caller(),
        );

        let (a, inclusion_proof) =
            get_account(&mut local_state.ledger, account_update.account_id());

        let acct = (a, inclusion_proof);

        let (transaction_commitment, full_transaction_commitment) = match &is_start {
            IsStart::No => (
                local_state.transaction_commitment,
                local_state.full_transaction_commitment,
            ),
            IsStart::Yes(start_data) | IsStart::Compute(start_data) => {
                let tx_commitment_on_start = commitment(remaining.calls.clone());
                let full_tx_commitment_on_start = full_commitment(
                    account_update.clone(),
                    start_data.memo_hash,
                    tx_commitment_on_start.clone(),
                );
                if let true = is_start_ {
                    (tx_commitment_on_start, full_tx_commitment_on_start)
                } else {
                    (
                        local_state.transaction_commitment,
                        local_state.full_transaction_commitment,
                    )
                }
            }
        };

        let local_state = LocalStateEnv {
            transaction_commitment,
            full_transaction_commitment,
            token_id: if let true = is_start_ {
                TokenId::default()
            } else {
                local_state.token_id
            },
            ..local_state
        };
        (
            (account_update, remaining, call_stack),
            account_update_forest,
            local_state,
            acct,
        )
    };

    let local_state = LocalStateEnv {
        stack_frame: remaining.clone(),
        call_stack,
        ..local_state
    };
    let local_state = local_state.add_new_failure_status_bucket();
    let account_is_new = check_account::<L>(
        account_update.public_key(),
        account_update.token_id(),
        (&a, &inclusion_proof),
    );

    let local_state = match Env::perform(Eff::CheckAccountPrecondition(
        account_update.clone(),
        a.clone(),
        account_is_new,
        local_state,
    )) {
        PerformResult::LocalState(local_state) => local_state,
        _ => unreachable!(),
    };

    let protocol_state_predicate_satisfied =
        if let PerformResult::Bool(protocol_state_predicate_satisfied) =
            Env::perform(Eff::CheckProtocolStatePrecondition(
                account_update.protocol_state_precondition(),
                global_state.clone(),
            ))
        {
            protocol_state_predicate_satisfied
        } else {
            unreachable!()
        };

    let local_state = local_state.add_check(
        TransactionFailure::ProtocolStatePreconditionUnsatisfied,
        protocol_state_predicate_satisfied,
    );

    let CheckAuthorizationResult {
        proof_verifies,
        signature_verifies,
    } = {
        let commitment = if let true = account_update.use_full_commitment() {
            local_state.full_transaction_commitment.clone()
        } else {
            local_state.transaction_commitment.clone()
        };
        account_update.check_authorization(commitment.0, account_update_forest)
    };

    let local_state = local_state.add_check(
        TransactionFailure::FeePayerNonceMustIncrease,
        account_update.increment_nonce() || !is_start_,
    );
    let local_state = local_state.add_check(
        TransactionFailure::FeePayerMustBeSigned,
        signature_verifies || !is_start_,
    );

    let precondition_has_constant_nonce = match account_update.account_precondition().nonce() {
        OrIgnore::Check(x) => x.is_constant(),
        OrIgnore::Ignore => false,
    };
    let increments_nonce_and_constrains_its_old_value =
        account_update.increment_nonce() && precondition_has_constant_nonce;
    let depends_on_the_fee_payers_nonce_and_isnt_the_fee_payer =
        account_update.use_full_commitment() && !is_start_;
    let does_not_use_a_signature = !signature_verifies;
    let local_state = local_state.add_check(
        TransactionFailure::ZkappCommandReplayCheckFailed,
        increments_nonce_and_constrains_its_old_value
            || depends_on_the_fee_payers_nonce_and_isnt_the_fee_payer
            || does_not_use_a_signature,
    );
    let a = Account {
        token_id: account_update.token_id(),
        ..a.clone()
    };
    let account_update_token_is_default = account_update.token_id() == TokenId::default();
    let account_is_untimed = !is_timed(&a);

    let timing = account_update.timing();
    let local_state = local_state.add_check(
        TransactionFailure::UpdateNotPermittedTimingExistingAccount,
        timing.is_keep() || (account_is_untimed && signature_verifies),
    );
    let timing = timing
        .into_map(Some)
        .set_or_keep(zkapp_command::Timing::of_account_timing(a.timing.clone()));

    // https://github.com/MinaProtocol/mina/blob/3fe924c80a4d01f418b69f27398f5f93eb652514/src/lib/transaction_logic/mina_transaction_logic.ml#L1197
    let vesting_period = match &timing {
        Some(timing) => timing.vesting_period,
        None => to_record(Timing::Untimed).vesting_period,
    };

    assert!(vesting_period > Slot::zero());
    let a = Account {
        timing: timing
            .map(|timing| timing.to_account_timing())
            .unwrap_or(Timing::Untimed),
        ..a.clone()
    };

    let (a, local_state) = {
        let balance_change = account_update.balance_change();
        let (balance, failed1) = a.balance.add_signed_amount_flagged(balance_change.clone());
        let local_state = local_state.add_check(TransactionFailure::Overflow, failed1);
        let local_state = {
            let account_creation_fee = constraint_constants.account_creation_fee;
            let (excess_minus_creation_fee, excess_update_failed) =
                local_state.excess.add_flagged(Signed::<Fee> {
                    magnitude: account_creation_fee,
                    sgn: Sgn::Neg,
                });
            let local_state = local_state.add_check(
                TransactionFailure::AmountInsufficientToCreateAccount,
                !(account_is_new && excess_update_failed),
            );
            LocalStateEnv {
                excess: if let true = account_is_new {
                    excess_minus_creation_fee
                } else {
                    local_state.excess
                },
                ..local_state
            }
        };
        let is_receiver = balance_change.is_pos();
        let local_state = {
            let controller = if let true = is_receiver {
                a.permissions.receive
            } else {
                a.permissions.send
            };
            let has_permission = controller_check(proof_verifies, signature_verifies, controller);
            local_state.add_check(
                TransactionFailure::UpdateNotPermittedBalance,
                has_permission || balance_change == Signed::<Amount>::zero(),
            )
        };
        let a = Account { balance, ..a };
        (a, local_state)
    };
    let txn_global_slot = global_state.protocol_state.global_slot_since_genesis;
    let (a, local_state) = {
        let (invalid_timing, timing) = match account_check_timing(&txn_global_slot, a.clone()) {
            (TimingValidation::InsufficientBalance(true), _) => {
                panic!("Did not propose a balance change at this timing check!")
            }
            (TimingValidation::InvalidTiming(true), timing) => (true, timing),
            (_, timing) => (false, timing),
        };
        let local_state = local_state.add_check(
            TransactionFailure::SourceMinimumBalanceViolation,
            !invalid_timing,
        );
        let a = Account {
            timing,
            ..a.clone()
        };
        (a, local_state)
    };
    let a = make_zkapp(a);

    let app_state = account_update.app_state();
    let keeping_app_state = app_state.iter().all(|x| x.is_keep());
    let changing_entire_app_state = app_state.iter().all(|x| x.is_set());
    let zkapp = a.zkapp.unwrap();

    let proved_state = if let true = keeping_app_state {
        zkapp.proved_state
    } else {
        if let true = proof_verifies {
            if let true = changing_entire_app_state {
                true
            } else {
                zkapp.proved_state
            }
        } else {
            false
        }
    };
    let zkapp = ZkAppAccount {
        proved_state,
        ..zkapp
    };
    let a = Account {
        zkapp: Some(zkapp.clone()),
        ..a
    };
    let has_permission =
        controller_check(proof_verifies, signature_verifies, a.permissions.edit_state);
    let local_state = local_state.add_check(
        TransactionFailure::UpdateNotPermittedAppState,
        keeping_app_state || has_permission,
    );
    let app_state: Vec<Fp> = app_state
        .iter()
        .zip(zkapp.app_state.iter())
        .map(|(x, y)| x.set_or_keep(*y))
        .collect();
    let app_state = [
        app_state[0],
        app_state[1],
        app_state[2],
        app_state[3],
        app_state[4],
        app_state[5],
        app_state[6],
        app_state[7],
    ];

    let zkapp = ZkAppAccount { app_state, ..zkapp };
    let a = Account {
        zkapp: Some(zkapp.clone()),
        ..a
    };

    let (a, local_state) = {
        let verification_key = account_update.verification_key();
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.set_verification_key,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedVerificationKey,
            verification_key.is_keep() || has_permission,
        );
        let verification_key = match zkapp.verification_key {
            Some(vk) => Some(verification_key.set_or_keep(vk)),
            None => None,
        };

        let zkapp = ZkAppAccount {
            verification_key,
            ..zkapp
        };
        let a = Account {
            zkapp: Some(zkapp),
            ..a
        };
        (a, local_state)
    };

    let (a, local_state) = {
        let sequence_events = account_update.sequence_events();
        let zkapp = a.zkapp.unwrap();
        let last_sequence_slot = zkapp.last_sequence_slot;
        let (sequence_state, last_sequence_slot) = update_sequence_state(
            zkapp.sequence_state,
            sequence_events.clone(),
            txn_global_slot,
            last_sequence_slot,
        );
        let is_empty = sequence_events.is_empty();
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.edit_sequence_state,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedAppState,
            is_empty || has_permission,
        );
        let zkapp = ZkAppAccount {
            sequence_state,
            last_sequence_slot,
            ..zkapp
        };
        let a = Account {
            zkapp: Some(zkapp),
            ..a
        };
        (a, local_state)
    };

    let a = unmake_zkapp(a);
    let (a, local_state) = {
        let zkapp_uri = account_update.zkapp_uri();
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.set_zkapp_uri,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedZkappUri,
            zkapp_uri.is_keep() || has_permission,
        );
        let zkapp = a.zkapp.map(|x| ZkAppAccount {
            zkapp_uri: zkapp_uri.set_or_keep(x.zkapp_uri),
            ..x
        });
        let a = Account { zkapp, ..a };
        (a, local_state)
    };

    let (a, local_state) = {
        let token_symbol = account_update.token_symbol();
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.set_token_symbol,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedTokenSymbol,
            token_symbol.is_keep() || has_permission,
        );
        let token_symbol = token_symbol.set_or_keep(a.token_symbol);
        let a = Account { token_symbol, ..a };
        (a, local_state)
    };

    let (a, local_state) = {
        let delegate = account_update.delegate();
        let base_delegate = if let true = account_is_new {
            Some(account_update.public_key())
        } else {
            a.delegate
        };
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.set_delegate,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedDelegate,
            delegate.is_keep() || (has_permission && account_update_token_is_default),
        );
        let delegate = base_delegate.map(|x| delegate.set_or_keep(x));
        let a = Account { delegate, ..a };
        (a, local_state)
    };

    let (a, local_state) = {
        let nonce = a.nonce;
        let increment_nonce = account_update.increment_nonce();
        let nonce = if let true = increment_nonce {
            nonce.incr()
        } else {
            nonce
        };
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.increment_nonce,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedNonce,
            !increment_nonce || has_permission,
        );
        let a = Account { nonce, ..a };
        (a, local_state)
    };

    let (a, local_state) = {
        let voting_for = account_update.voting_for();
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.set_voting_for,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedVotingFor,
            voting_for.is_keep() || has_permission,
        );
        let voting_for = voting_for.set_or_keep(a.voting_for);
        let a = Account { voting_for, ..a };
        (a, local_state)
    };

    let a = {
        let new_hash = {
            let old_hash = a.receipt_chain_hash;
            if let true = signature_verifies {
                let elt = ZkAppCommandElt::ZkAppCommandCommitment(
                    local_state.full_transaction_commitment.clone(),
                );
                ReceiptChainHash(cons_zkapp_command_commitment(
                    local_state.account_update_index.as_u32(),
                    elt,
                    old_hash,
                ))
            } else {
                old_hash
            }
        };
        Account {
            receipt_chain_hash: new_hash,
            ..a
        }
    };

    let (a, local_state) = {
        let permissions = account_update.permissions();
        let has_permission = controller_check(
            proof_verifies,
            signature_verifies,
            a.permissions.set_permissions,
        );
        let local_state = local_state.add_check(
            TransactionFailure::UpdateNotPermittedPermissions,
            permissions.is_keep() || has_permission,
        );
        let permissions = permissions.set_or_keep(a.permissions);
        let a = Account { permissions, ..a };
        (a, local_state)
    };

    let a = match Env::perform(Eff::<L>::InitAccount(account_update.clone(), a)) {
        PerformResult::Account(a) => a,
        _ => unreachable!(),
    };

    let local_delta = account_update.balance_change().negate();
    let (new_local_fee_excess, overflowed) = {
        let curr_token = local_state.token_id.clone();
        let curr_is_default = curr_token == TokenId::default();
        assert!(curr_is_default);
        assert!(!is_start_ || (account_update_token_is_default && local_delta.is_pos()));
        let (new_local_fee_excess, overflow) = local_state.excess.add_flagged(Signed::<Fee> {
            magnitude: Fee::from_u64(local_delta.magnitude.as_u64()),
            sgn: local_delta.sgn,
        });

        let new_local_fee_excess = if account_update_token_is_default {
            new_local_fee_excess
        } else {
            local_state.excess.clone()
        };
        (
            new_local_fee_excess,
            account_update_token_is_default && overflow,
        )
    };
    let local_state = LocalStateEnv {
        excess: new_local_fee_excess,
        ..local_state
    };
    let mut local_state =
        local_state.add_check(TransactionFailure::LocalExcessOverflow, !overflowed);

    let new_ledger = set_account(&mut local_state.ledger, (a, &inclusion_proof));
    let is_last_account_update = remaining.calls.is_empty();
    let local_state = LocalStateEnv {
        ledger: new_ledger.clone(),
        transaction_commitment: if is_last_account_update {
            ReceiptChainHash(Fp::zero())
        } else {
            local_state.transaction_commitment
        },
        full_transaction_commitment: if is_last_account_update {
            ReceiptChainHash(Fp::zero())
        } else {
            local_state.full_transaction_commitment
        },
        ..local_state
    };

    let valid_fee_excess = {
        let delta_settled = local_state.excess == Signed::<Fee>::zero();
        !is_last_account_update || delta_settled
    };
    let local_state = local_state.add_check(TransactionFailure::InvalidFeeExcess, valid_fee_excess);
    let update_local_excess = is_start_ || is_last_account_update;
    let update_global_state = update_local_excess && local_state.success;

    let (global_state, global_excess_update_failed, update_global_state) = {
        let amt = global_state.fee_excess.clone();
        let (res, overflow) = amt.add_flagged(local_state.excess.clone());
        let global_excess_update_failed = update_global_state && overflow;
        let update_global_state = update_global_state && !overflow;
        let new_amt = if update_global_state { res } else { amt };
        (
            GlobalState {
                fee_excess: new_amt,
                ..global_state.clone()
            },
            global_excess_update_failed,
            update_global_state,
        )
    };
    let local_state = LocalStateEnv {
        excess: if update_local_excess {
            Signed::<Fee>::zero()
        } else {
            local_state.excess
        },
        ..local_state
    };
    let local_state = local_state.add_check(
        TransactionFailure::GlobalExcessOverflow,
        !global_excess_update_failed,
    );
    assert_with_failure_status_tbl(
        !is_start_ || local_state.success,
        local_state.failure_status_tbl.clone(),
    );
    let global_state = set_ledger(
        update_global_state,
        global_state,
        local_state.ledger.clone(),
    );
    let local_state = LocalStateEnv {
        token_id: if is_last_account_update {
            TokenId::default()
        } else {
            local_state.token_id
        },
        ledger: if is_last_account_update {
            L::empty(constraint_constants.ledger_depth as usize)
            // empty_ledger(constraint_constants.ledger_depth as usize)
        } else {
            local_state.ledger
        },
        success: if is_last_account_update {
            true
        } else {
            local_state.success
        },
        account_update_index: if is_last_account_update {
            Index::zero()
        } else {
            local_state.account_update_index.incr()
        },
        ..local_state
    };
    (global_state, local_state)
}
