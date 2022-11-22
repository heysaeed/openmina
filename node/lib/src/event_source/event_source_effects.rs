use crate::action::CheckTimeoutsAction;
use crate::p2p::connection::outgoing::{
    P2pConnectionOutgoingErrorAction, P2pConnectionOutgoingSuccessAction,
};
use crate::p2p::pubsub::P2pPubsubBytesReceivedAction;
use crate::rpc::{
    RpcGlobalStateGetAction, RpcP2pConnectionOutgoingInitAction, RpcP2pPubsubMessagePublishAction,
    RpcRequest,
};
use crate::{Service, Store};

use super::{
    Event, EventSourceAction, EventSourceActionWithMeta, EventSourceNewEventAction,
    P2pConnectionEvent, P2pEvent, P2pPubsubEvent,
};

pub fn event_source_effects<S: Service>(store: &mut Store<S>, action: EventSourceActionWithMeta) {
    let (action, meta) = action.split();
    match action {
        EventSourceAction::ProcessEvents(_) => {
            // process max 1024 events at a time.
            for _ in 0..1024 {
                match store.service.next_event() {
                    Some(event) => {
                        store.dispatch(EventSourceNewEventAction { event });
                    }
                    None => break,
                }
            }
            store.dispatch(CheckTimeoutsAction {});
        }
        EventSourceAction::NewEvent(content) => match content.event {
            Event::P2p(e) => match e {
                P2pEvent::Connection(e) => match e {
                    P2pConnectionEvent::OutgoingInit(peer_id, result) => match result {
                        Err(error) => {
                            store.dispatch(P2pConnectionOutgoingErrorAction { peer_id, error });
                        }
                        Ok(_) => {
                            store.dispatch(P2pConnectionOutgoingSuccessAction { peer_id });
                        }
                    },
                },
                P2pEvent::Pubsub(e) => match e {
                    P2pPubsubEvent::BytesReceived {
                        author,
                        sender,
                        topic,
                        bytes,
                    } => {
                        store.dispatch(P2pPubsubBytesReceivedAction {
                            author,
                            sender,
                            topic,
                            bytes,
                        });
                    }
                },
                P2pEvent::Rpc(e) => {
                    shared::log::warn!(meta.time(); kind = "UnhandledP2pRpcEvent", event = format!("{:?}", e));
                }
            },
            Event::Rpc(rpc_id, e) => match e {
                RpcRequest::GetState => {
                    store.dispatch(RpcGlobalStateGetAction { rpc_id });
                }
                RpcRequest::P2pConnectionOutgoing(opts) => {
                    store.dispatch(RpcP2pConnectionOutgoingInitAction { rpc_id, opts });
                }
                RpcRequest::P2pPubsubPublish(topic, message) => {
                    store.dispatch(RpcP2pPubsubMessagePublishAction {
                        rpc_id,
                        topic,
                        message,
                    });
                }
            },
        },
        EventSourceAction::WaitTimeout(_) => {
            store.dispatch(CheckTimeoutsAction {});
        }
        EventSourceAction::WaitForEvents(_) => {}
    }
}
