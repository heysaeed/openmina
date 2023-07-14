use mina_p2p_messages::v2::StateHash;
use serde::{Deserialize, Serialize};
use shared::block::ArcBlockWithHash;

use crate::p2p::channels::rpc::P2pRpcId;
use crate::p2p::PeerId;

use super::{
    sync::ledger::{
        PeerLedgerQueryError, TransitionFrontierSyncLedgerAction, TransitionFrontierSyncLedgerState,
    },
    TransitionFrontierSyncState,
};

pub type TransitionFrontierActionWithMeta = redux::ActionWithMeta<TransitionFrontierAction>;
pub type TransitionFrontierActionWithMetaRef<'a> =
    redux::ActionWithMeta<&'a TransitionFrontierAction>;

#[derive(derive_more::From, Serialize, Deserialize, Debug, Clone)]
pub enum TransitionFrontierAction {
    SyncInit(TransitionFrontierSyncInitAction),
    SyncBestTipUpdate(TransitionFrontierSyncBestTipUpdateAction),
    SyncLedgerRootPending(TransitionFrontierSyncLedgerRootPendingAction),
    SyncLedgerRootSuccess(TransitionFrontierSyncLedgerRootSuccessAction),
    SyncBlocksPending(TransitionFrontierSyncBlocksPendingAction),
    SyncBlocksPeersQuery(TransitionFrontierSyncBlocksPeersQueryAction),
    SyncBlocksPeerQueryInit(TransitionFrontierSyncBlocksPeerQueryInitAction),
    SyncBlocksPeerQueryRetry(TransitionFrontierSyncBlocksPeerQueryRetryAction),
    SyncBlocksPeerQueryPending(TransitionFrontierSyncBlocksPeerQueryPendingAction),
    SyncBlocksPeerQueryError(TransitionFrontierSyncBlocksPeerQueryErrorAction),
    SyncBlocksPeerQuerySuccess(TransitionFrontierSyncBlocksPeerQuerySuccessAction),
    SyncBlocksFetchSuccess(TransitionFrontierSyncBlocksFetchSuccessAction),
    SyncBlocksNextApplyInit(TransitionFrontierSyncBlocksNextApplyInitAction),
    SyncBlocksApplyPending(TransitionFrontierSyncBlocksApplyPendingAction),
    SyncBlocksApplySuccess(TransitionFrontierSyncBlocksApplySuccessAction),
    SyncBlocksSuccess(TransitionFrontierSyncBlocksSuccessAction),

    Synced(TransitionFrontierSyncedAction),

    SyncLedger(TransitionFrontierSyncLedgerAction),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncInitAction {
    pub best_tip: ArcBlockWithHash,
    pub root_block: ArcBlockWithHash,
    pub blocks_inbetween: Vec<StateHash>,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncInitAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        !state.transition_frontier.sync.is_pending()
            && !state.transition_frontier.sync.is_synced()
            && state
                .transition_frontier
                .best_tip()
                .map_or(true, |tip| self.best_tip.hash != tip.hash)
            && state
                .consensus
                .best_tip()
                .map_or(false, |tip| &self.best_tip.hash == tip.hash)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBestTipUpdateAction {
    pub best_tip: ArcBlockWithHash,
    pub root_block: ArcBlockWithHash,
    pub blocks_inbetween: Vec<StateHash>,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBestTipUpdateAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        (state.transition_frontier.sync.is_pending() || state.transition_frontier.sync.is_synced())
            && state
                .transition_frontier
                .best_tip()
                .map_or(true, |tip| self.best_tip.hash != tip.hash)
            && state
                .transition_frontier
                .sync
                .best_tip()
                .map_or(true, |tip| self.best_tip.hash != tip.hash)
            && state
                .consensus
                .best_tip()
                .map_or(true, |tip| &self.best_tip.hash == tip.hash)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncLedgerRootPendingAction {}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncLedgerRootPendingAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        matches!(
            state.transition_frontier.sync,
            TransitionFrontierSyncState::Init { .. }
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncLedgerRootSuccessAction {}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncLedgerRootSuccessAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        matches!(
            state.transition_frontier.sync,
            TransitionFrontierSyncState::RootLedgerPending {
                root_ledger: TransitionFrontierSyncLedgerState::Success { .. },
                ..
            }
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksPendingAction {}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksPendingAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        matches!(
            state.transition_frontier.sync,
            TransitionFrontierSyncState::RootLedgerSuccess { .. }
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksPeersQueryAction {}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksPeersQueryAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        let peers_available = state
            .p2p
            .ready_peers_iter()
            .any(|(_, p)| p.channels.rpc.can_send_request());
        let sync = &state.transition_frontier.sync;
        peers_available
            && (sync.blocks_fetch_next().is_some()
                || sync.blocks_fetch_retry_iter().next().is_some())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksPeerQueryInitAction {
    pub hash: StateHash,
    pub peer_id: PeerId,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksPeerQueryInitAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        let check_next_hash = state
            .transition_frontier
            .sync
            .blocks_fetch_next()
            .map_or(false, |expected| expected == self.hash);

        let check_peer_available = state
            .p2p
            .get_ready_peer(&self.peer_id)
            .and_then(|p| {
                let sync_best_tip = state.transition_frontier.sync.best_tip()?;
                let peer_best_tip = p.best_tip.as_ref()?;
                Some(p).filter(|_| sync_best_tip.hash == peer_best_tip.hash)
            })
            .map_or(false, |p| p.channels.rpc.can_send_request());

        check_next_hash && check_peer_available
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksPeerQueryRetryAction {
    pub hash: StateHash,
    pub peer_id: PeerId,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksPeerQueryRetryAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        let check_next_hash = state
            .transition_frontier
            .sync
            .blocks_fetch_retry_iter()
            .next()
            .map_or(false, |expected| expected == self.hash);

        let check_peer_available = state
            .p2p
            .get_ready_peer(&self.peer_id)
            .and_then(|p| {
                let sync_best_tip = state.transition_frontier.sync.best_tip()?;
                let peer_best_tip = p.best_tip.as_ref()?;
                Some(p).filter(|_| sync_best_tip.hash == peer_best_tip.hash)
            })
            .map_or(false, |p| p.channels.rpc.can_send_request());

        check_next_hash && check_peer_available
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksPeerQueryPendingAction {
    pub hash: StateHash,
    pub peer_id: PeerId,
    pub rpc_id: P2pRpcId,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksPeerQueryPendingAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        state
            .transition_frontier
            .sync
            .block_state(&self.hash)
            .map_or(false, |b| b.is_fetch_init_from_peer(&self.peer_id))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksPeerQueryErrorAction {
    pub peer_id: PeerId,
    pub rpc_id: P2pRpcId,
    pub error: PeerLedgerQueryError,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksPeerQueryErrorAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        state
            .transition_frontier
            .sync
            .blocks_iter()
            .any(|s| s.is_fetch_pending_from_peer(&self.peer_id, self.rpc_id))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksPeerQuerySuccessAction {
    pub peer_id: PeerId,
    pub rpc_id: P2pRpcId,
    pub response: ArcBlockWithHash,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksPeerQuerySuccessAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        state
            .transition_frontier
            .sync
            .block_state(&self.response.hash)
            .filter(|s| s.is_fetch_pending_from_peer(&self.peer_id, self.rpc_id))
            .map_or(false, |s| s.block_hash() == &self.response.hash)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksFetchSuccessAction {
    pub hash: StateHash,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksFetchSuccessAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        state
            .transition_frontier
            .sync
            .block_state(&self.hash)
            .map_or(false, |s| s.fetch_pending_fetched_block().is_some())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksNextApplyInitAction {}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksNextApplyInitAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        state.transition_frontier.sync.blocks_apply_next().is_some()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksApplyPendingAction {
    pub hash: StateHash,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksApplyPendingAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        state
            .transition_frontier
            .sync
            .blocks_apply_next()
            .map_or(false, |(b, _)| b.hash == self.hash)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksApplySuccessAction {
    pub hash: StateHash,
}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksApplySuccessAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        state
            .transition_frontier
            .sync
            .blocks_apply_pending()
            .map_or(false, |b| b.hash == self.hash)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncBlocksSuccessAction {}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncBlocksSuccessAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        match &state.transition_frontier.sync {
            TransitionFrontierSyncState::BlocksPending { chain, .. } => {
                chain.iter().all(|v| v.is_apply_success())
            }
            _ => false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransitionFrontierSyncedAction {}

impl redux::EnablingCondition<crate::State> for TransitionFrontierSyncedAction {
    fn is_enabled(&self, state: &crate::State) -> bool {
        matches!(
            state.transition_frontier.sync,
            TransitionFrontierSyncState::BlocksSuccess { .. }
        )
    }
}

macro_rules! impl_into_global_action {
    ($a:ty) => {
        impl From<$a> for crate::Action {
            fn from(value: $a) -> Self {
                Self::TransitionFrontier(value.into())
            }
        }
    };
}

impl_into_global_action!(TransitionFrontierSyncInitAction);
impl_into_global_action!(TransitionFrontierSyncBestTipUpdateAction);
impl_into_global_action!(TransitionFrontierSyncLedgerRootPendingAction);
impl_into_global_action!(TransitionFrontierSyncLedgerRootSuccessAction);
impl_into_global_action!(TransitionFrontierSyncBlocksPendingAction);
impl_into_global_action!(TransitionFrontierSyncBlocksPeersQueryAction);
impl_into_global_action!(TransitionFrontierSyncBlocksPeerQueryInitAction);
impl_into_global_action!(TransitionFrontierSyncBlocksPeerQueryRetryAction);
impl_into_global_action!(TransitionFrontierSyncBlocksPeerQueryPendingAction);
impl_into_global_action!(TransitionFrontierSyncBlocksPeerQueryErrorAction);
impl_into_global_action!(TransitionFrontierSyncBlocksPeerQuerySuccessAction);
impl_into_global_action!(TransitionFrontierSyncBlocksFetchSuccessAction);
impl_into_global_action!(TransitionFrontierSyncBlocksNextApplyInitAction);
impl_into_global_action!(TransitionFrontierSyncBlocksApplyPendingAction);
impl_into_global_action!(TransitionFrontierSyncBlocksApplySuccessAction);
impl_into_global_action!(TransitionFrontierSyncBlocksSuccessAction);
impl_into_global_action!(TransitionFrontierSyncedAction);
