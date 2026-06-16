//! `JsLibp2pProvider` — the first impl of `NetworkProvider`.
//!
//! Wraps the existing js-libp2p instance (which lives in
//! `assets/src/js-bridge.js`) behind the `NetworkProvider` seam.
//! Application code never holds this concrete type; it holds
//! `Box<dyn NetworkProvider>`. Substrate swaps (`RustLibp2pProvider`,
//! `RemoteServerProvider`) drop in via the same trait.
//!
//! ## Phase 2b (this commit)
//!
//! The provider takes five JS callbacks at construction. The bridge
//! defines them on top of the existing libp2p instance and hands them
//! to `roam_net_init` once libp2p is ready. Outgoing publishes flow
//! Rust → callback → libp2p; incoming events stay in JS for now
//! (handled in 2c).
//!
//! Per-call cost: one wasm→JS function dispatch + arg marshaling.
//! No new crypto, no new IPC — same wire as today.

use crate::net::{NetError, NetEvent, NetworkProvider, PeerId, Topic};

#[cfg(target_arch = "wasm32")]
use js_sys::Function;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

/// Five JS callbacks the bridge supplies. Each one is a plain JS
/// function; the provider invokes them through `js_sys::Function`.
/// On non-wasm targets we keep `JsLibp2pProvider` compilable for unit
/// tests by replacing the function fields with no-op closures over a
/// shared identity string.
#[cfg(target_arch = "wasm32")]
pub struct JsLibp2pProvider {
    self_peer_id: PeerId,
    publish: Function,
    subscribe: Function,
    unsubscribe: Function,
    drain_events: Function,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct JsLibp2pProvider {
    self_peer_id: PeerId,
}

#[cfg(target_arch = "wasm32")]
impl JsLibp2pProvider {
    /// Construct from the JS callbacks. `self_peer_id_fn` is invoked
    /// once here to lock the identity; the other four are stored and
    /// invoked per operation.
    pub fn new(
        self_peer_id_fn: Function,
        publish: Function,
        subscribe: Function,
        unsubscribe: Function,
        drain_events: Function,
    ) -> Result<Self, JsValue> {
        let id_val = self_peer_id_fn.call0(&JsValue::NULL)?;
        let id_str = id_val
            .as_string()
            .ok_or_else(|| JsValue::from_str("js_net_self_peer_id did not return a string"))?;
        Ok(Self {
            self_peer_id: PeerId(id_str),
            publish,
            subscribe,
            unsubscribe,
            drain_events,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl JsLibp2pProvider {
    /// Native (test) constructor. No-op publish/subscribe; identity
    /// is whatever the caller passes in.
    pub fn new_for_tests(self_peer_id: PeerId) -> Self {
        Self { self_peer_id }
    }
}

#[cfg(target_arch = "wasm32")]
impl NetworkProvider for JsLibp2pProvider {
    fn identity(&self) -> PeerId {
        self.self_peer_id.clone()
    }

    fn publish(&mut self, topic: &Topic, bytes: &[u8]) -> Result<(), NetError> {
        let topic_arg = JsValue::from_str(&topic.0);
        let bytes_arg = js_sys::Uint8Array::from(bytes);
        self.publish
            .call2(&JsValue::NULL, &topic_arg, &bytes_arg)
            .map_err(|e| NetError::PublishFailed {
                topic: topic.clone(),
                reason: js_value_message(&e),
            })?;
        Ok(())
    }

    fn subscribe(&mut self, topic: &Topic) -> Result<(), NetError> {
        let topic_arg = JsValue::from_str(&topic.0);
        self.subscribe
            .call1(&JsValue::NULL, &topic_arg)
            .map_err(|e| NetError::SubscribeFailed {
                topic: topic.clone(),
                reason: js_value_message(&e),
            })?;
        Ok(())
    }

    fn unsubscribe(&mut self, topic: &Topic) -> Result<(), NetError> {
        let topic_arg = JsValue::from_str(&topic.0);
        self.unsubscribe
            .call1(&JsValue::NULL, &topic_arg)
            .map_err(|e| NetError::SubscribeFailed {
                topic: topic.clone(),
                reason: js_value_message(&e),
            })?;
        Ok(())
    }

    fn poll_events(&mut self) -> Vec<NetEvent> {
        // Phase 2c: drain the JS-side queue. Format on the wire (set
        // by `net-shim.js attach`):
        //
        //   [{ topic: string, from: string, bytes: number[], at_ms: number }, ...]
        //
        // PeerUp/PeerDown/SubscriptionChange/Error aren't queued yet
        // — they're added once we attach the corresponding libp2p
        // event listeners on the shim side.
        let raw = match self.drain_events.call0(&JsValue::NULL) {
            Ok(v) => v,
            Err(e) => {
                return vec![NetEvent::Error(NetError::ProviderInternal {
                    reason: js_value_message(&e),
                })];
            }
        };
        let raw_str = match raw.as_string() {
            Some(s) => s,
            None => {
                return vec![NetEvent::Error(NetError::ProviderInternal {
                    reason: "drain_events did not return a string".to_string(),
                })];
            }
        };
        if raw_str.is_empty() || raw_str == "[]" {
            return Vec::new();
        }
        match serde_json::from_str::<Vec<MessageWire>>(&raw_str) {
            Ok(items) => items
                .into_iter()
                .map(|m| NetEvent::Message {
                    topic: Topic(m.topic),
                    from: PeerId(m.from),
                    bytes: m.bytes,
                    at_ms: m.at_ms,
                })
                .collect(),
            Err(e) => vec![NetEvent::Error(NetError::ProviderInternal {
                reason: format!("drain_events JSON parse failed: {e}"),
            })],
        }
    }
}

/// Wire shape for one queued message from `net-shim.js`. Numeric
/// array for bytes; no base64 dep.
#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
struct MessageWire {
    topic: String,
    from: String,
    bytes: Vec<u8>,
    at_ms: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl NetworkProvider for JsLibp2pProvider {
    fn identity(&self) -> PeerId {
        self.self_peer_id.clone()
    }
    fn publish(&mut self, _topic: &Topic, _bytes: &[u8]) -> Result<(), NetError> {
        Ok(())
    }
    fn subscribe(&mut self, _topic: &Topic) -> Result<(), NetError> {
        Ok(())
    }
    fn unsubscribe(&mut self, _topic: &Topic) -> Result<(), NetError> {
        Ok(())
    }
    fn poll_events(&mut self) -> Vec<NetEvent> {
        Vec::new()
    }
}

#[cfg(target_arch = "wasm32")]
fn js_value_message(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            js_sys::Reflect::get(e, &JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    #[test]
    fn provider_returns_supplied_identity() {
        let p = JsLibp2pProvider::new_for_tests(PeerId("12D3KooWtest".into()));
        assert_eq!(p.identity().0, "12D3KooWtest");
    }

    #[test]
    fn placeholder_methods_dont_panic() {
        let mut p = JsLibp2pProvider::new_for_tests(PeerId("12D3KooWtest".into()));
        let topic = Topic("roam-positions/v1".into());
        assert!(p.publish(&topic, &[1, 2, 3]).is_ok());
        assert!(p.subscribe(&topic).is_ok());
        assert!(p.unsubscribe(&topic).is_ok());
        assert!(p.poll_events().is_empty());
    }

    #[test]
    fn provider_is_object_safe_through_box() {
        let p: Box<dyn NetworkProvider> = Box::new(JsLibp2pProvider::new_for_tests(PeerId(
            "12D3KooWtest".into(),
        )));
        assert_eq!(p.identity().0, "12D3KooWtest");
    }
}
