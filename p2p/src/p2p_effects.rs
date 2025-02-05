use redux::{ActionMeta, ActionWithMeta};

use crate::{
    channels::{P2pChannelsAction, P2pChannelsService},
    connection::{
        outgoing::P2pConnectionOutgoingAction, P2pConnectionAction, P2pConnectionService,
    },
    disconnection::P2pDisconnectionService,
    P2pAction, P2pCryptoService, P2pMioService, P2pNetworkKadKey, P2pNetworkKademliaAction,
    P2pNetworkSelectAction, P2pNetworkService, P2pStore, PeerId,
};

pub fn p2p_timeout_effects<Store, S>(store: &mut Store, meta: &ActionMeta)
where
    Store: P2pStore<S>,
{
    p2p_connection_timeouts(store, meta);
    store.dispatch(P2pConnectionOutgoingAction::RandomInit);

    p2p_try_reconnect_disconnected_peers(store, meta.time());

    p2p_discovery(store, meta);
    p2p_select_timeouts(store, meta);

    let state = store.state();
    for (peer_id, id) in state.peer_rpc_timeouts(meta.time()) {
        store.dispatch(crate::channels::rpc::P2pChannelsRpcAction::Timeout { peer_id, id });
    }
}

fn p2p_select_timeouts<Store, S>(store: &mut Store, meta: &ActionMeta)
where
    Store: P2pStore<S>,
{
    let now = meta.time();
    let timeouts = &store.state().config.timeouts;
    let select_auth_timeouts: Vec<_> = store
        .state()
        .network
        .scheduler
        .connections
        .iter()
        .filter_map(|(sock_addr, state)| {
            if state.select_auth.is_timed_out(now, timeouts) {
                Some(*sock_addr)
            } else {
                None
            }
        })
        .collect();

    let select_mux_timeouts: Vec<_> = store
        .state()
        .network
        .scheduler
        .connections
        .iter()
        .filter_map(|(sock_addr, state)| {
            if state.select_mux.is_timed_out(now, timeouts) {
                Some(*sock_addr)
            } else {
                None
            }
        })
        .collect();

    let select_stream_timeouts: Vec<_> = store
        .state()
        .network
        .scheduler
        .connections
        .iter()
        .flat_map(|(sock_addr, state)| {
            state.streams.iter().filter_map(|(stream_id, stream)| {
                if stream.select.is_timed_out(now, timeouts) {
                    Some((*sock_addr, *stream_id))
                } else {
                    None
                }
            })
        })
        .collect();

    for addr in select_auth_timeouts {
        store.dispatch(P2pNetworkSelectAction::Timeout {
            addr,
            kind: crate::SelectKind::Authentication,
        });
    }

    for addr in select_mux_timeouts {
        store.dispatch(P2pNetworkSelectAction::Timeout {
            addr,
            kind: crate::SelectKind::MultiplexingNoPeerId,
        });
    }

    for (addr, stream_id) in select_stream_timeouts {
        // TODO: better solution for PeerId
        let dummy = PeerId::from_bytes([0u8; 32]);

        store.dispatch(P2pNetworkSelectAction::Timeout {
            addr,
            kind: crate::SelectKind::Stream(dummy, stream_id),
        });
    }
}

fn p2p_connection_timeouts<Store, S>(store: &mut Store, meta: &ActionMeta)
where
    Store: P2pStore<S>,
{
    use crate::connection::incoming::P2pConnectionIncomingAction;

    let now = meta.time();
    let timeouts = &store.state().config.timeouts;
    let p2p_connection_timeouts: Vec<_> = store
        .state()
        .peers
        .iter()
        .filter_map(|(peer_id, peer)| {
            let s = peer.status.as_connecting()?;
            match s.is_timed_out(now, timeouts) {
                true => Some((*peer_id, s.as_outgoing().is_some())),
                false => None,
            }
        })
        .collect();

    for (peer_id, is_outgoing) in p2p_connection_timeouts {
        match is_outgoing {
            true => store.dispatch(P2pConnectionOutgoingAction::Timeout { peer_id }),
            false => store.dispatch(P2pConnectionIncomingAction::Timeout { peer_id }),
        };
    }
}

fn p2p_try_reconnect_disconnected_peers<Store, S>(store: &mut Store, now: redux::Timestamp)
where
    Store: P2pStore<S>,
{
    if store.state().already_has_min_peers() {
        return;
    }
    let timeouts = &store.state().config.timeouts;
    let reconnect_actions: Vec<_> = store
        .state()
        .peers
        .iter()
        .filter_map(|(_, p)| {
            if p.can_reconnect(now, timeouts) {
                p.dial_opts.clone()
            } else {
                None
            }
        })
        .map(|opts| P2pConnectionOutgoingAction::Reconnect { opts, rpc_id: None })
        .collect();
    for action in reconnect_actions {
        store.dispatch(action);
    }
}

fn p2p_discovery<Store, S>(store: &mut Store, meta: &redux::ActionMeta)
where
    Store: P2pStore<S>,
{
    let now = meta.time();
    let state = store.state();
    let config = &state.config;
    if !config.peer_discovery {
        return;
    }
    // ask initial peers
    if let Some(_d) = config.timeouts.initial_peers {
        // TODO: use RPC to ask initial peers
    }

    if let Some(discovery_state) = state.network.scheduler.discovery_state() {
        let key = state.my_id();
        if discovery_state
            .routing_table
            .closest_peers(&P2pNetworkKadKey::from(&key))
            .any(|_| true)
            && discovery_state.status.can_bootstrap(now, &config.timeouts)
        {
            store.dispatch(P2pNetworkKademliaAction::StartBootstrap { key });
        }
    }
}

pub fn p2p_effects<Store, S>(store: &mut Store, action: ActionWithMeta<P2pAction>)
where
    Store: P2pStore<S>,
    Store::Service: P2pConnectionService
        + P2pDisconnectionService
        + P2pChannelsService
        + P2pMioService
        + P2pCryptoService
        + P2pNetworkService,
{
    let (action, meta) = action.split();
    match action {
        P2pAction::Initialization(_) => {
            // Noop
        }
        P2pAction::Connection(action) => match action {
            P2pConnectionAction::Outgoing(action) => action.effects(&meta, store),
            P2pConnectionAction::Incoming(action) => action.effects(&meta, store),
        },
        P2pAction::Disconnection(action) => action.effects(&meta, store),
        P2pAction::Discovery(action) => action.effects(&meta, store),
        P2pAction::Identify(action) => action.effects(&meta, store),
        P2pAction::Channels(action) => match action {
            P2pChannelsAction::MessageReceived(action) => action.effects(&meta, store),
            P2pChannelsAction::BestTip(action) => action.effects(&meta, store),
            P2pChannelsAction::Snark(action) => action.effects(&meta, store),
            P2pChannelsAction::SnarkJobCommitment(action) => action.effects(&meta, store),
            P2pChannelsAction::Rpc(action) => action.effects(&meta, store),
        },
        P2pAction::Peer(action) => action.effects(&meta, store),
        P2pAction::Network(action) => action.effects(&meta, store),
    }
}
