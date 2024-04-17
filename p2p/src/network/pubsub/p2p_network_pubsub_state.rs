use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
};

use serde::{Deserialize, Serialize};

use crate::{token::BroadcastAlgorithm, PeerId, StreamId};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct P2pNetworkPubsubState {
    pub clients: BTreeMap<PeerId, P2pNetworkPubsubClientState>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct P2pNetworkPubsubClientState {
    pub protocol: BroadcastAlgorithm,
    pub addr: SocketAddr,
    pub stream_id: StreamId,
    pub topics: BTreeSet<String>,
}
