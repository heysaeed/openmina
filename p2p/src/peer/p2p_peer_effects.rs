use redux::ActionMeta;

use crate::channels::{
    best_tip::P2pChannelsBestTipAction, rpc::P2pChannelsRpcAction, snark::P2pChannelsSnarkAction,
    snark_job_commitment::P2pChannelsSnarkJobCommitmentAction, ChannelId,
};

use super::P2pPeerReadyAction;

impl P2pPeerReadyAction {
    pub fn effects<Store, S>(self, _: &ActionMeta, store: &mut Store)
    where
        Store: crate::P2pStore<S>,
        P2pChannelsBestTipAction: redux::EnablingCondition<S>,
        P2pChannelsSnarkAction: redux::EnablingCondition<S>,
        P2pChannelsSnarkJobCommitmentAction: redux::EnablingCondition<S>,
        P2pChannelsRpcAction: redux::EnablingCondition<S>,
    {
        let peer_id = self.peer_id;
        // Dispatches can be done without a loop, but inside we do
        // exhaustive matching so that we don't miss any channels.
        for id in ChannelId::iter_all() {
            match id {
                ChannelId::BestTipPropagation => {
                    store.dispatch(P2pChannelsBestTipAction::Init { peer_id });
                }
                ChannelId::SnarkPropagation => {
                    store.dispatch(P2pChannelsSnarkAction::Init { peer_id });
                }
                ChannelId::SnarkJobCommitmentPropagation => {
                    store.dispatch(P2pChannelsSnarkJobCommitmentAction::Init { peer_id });
                }
                ChannelId::Rpc => {
                    store.dispatch(P2pChannelsRpcAction::Init { peer_id });
                }
            }
        }
    }
}
