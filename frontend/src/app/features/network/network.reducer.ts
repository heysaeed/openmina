import { ActionReducer, combineReducers } from '@ngrx/store';
import { NetworkState } from '@network/network.state';

import * as fromMessages from '@network/messages/network-messages.reducer';
import * as fromConnections from '@network/connections/network-connections.reducer';
import * as fromBlocks from '@network/blocks/network-blocks.reducer';

import { NetworkMessagesAction, NetworkMessagesActions } from '@network/messages/network-messages.actions';
import { NetworkConnectionsAction, NetworkConnectionsActions } from '@network/connections/network-connections.actions';
import { NetworkBlocksAction, NetworkBlocksActions } from '@network/blocks/network-blocks.actions';
import { NetworkNodeDhtAction, NetworkNodeDhtActions } from '@network/node-dht/network-node-dht.actions';
import { topologyReducer } from '@network/splits/dashboard-splits.reducer';
import { networkDhtReducer } from '@network/node-dht/network-node-dht.reducer';

export type NetworkActions =
  NetworkMessagesActions
  & NetworkConnectionsActions
  & NetworkBlocksActions
  & NetworkNodeDhtActions;
export type NetworkAction =
  NetworkMessagesAction
  & NetworkConnectionsAction
  & NetworkBlocksAction
  & NetworkNodeDhtAction;

export const networkReducer: ActionReducer<NetworkState, NetworkActions> = combineReducers<NetworkState, NetworkActions>({
  messages: fromMessages.reducer,
  connections: fromConnections.reducer,
  blocks: fromBlocks.reducer,
  splits: topologyReducer,
  nodeDht: networkDhtReducer
});
