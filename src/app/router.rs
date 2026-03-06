//! Session routing for ACP messages.
//!
//! This module provides the `SessionRouter` struct for routing ACP messages
//! to per-session channels based on the session ID in the message params.

use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Buffer size for per-session message channels.
const CHANNEL_BUFFER_SIZE: usize = 100;

/// Routes ACP messages to per-session channels.
///
/// The `SessionRouter` maintains a mapping of session IDs to message channels,
/// allowing incoming ACP messages to be delivered to the appropriate session handler.
pub struct SessionRouter {
    routes: HashMap<String, mpsc::Sender<Value>>,
}

impl SessionRouter {
    /// Create a new empty session router.
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
        }
    }

    /// Register a new session and return a receiver for its messages.
    ///
    /// Creates a new channel for the session and returns the receiver end.
    /// The router keeps the sender end for routing messages.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The unique identifier for the session
    ///
    /// # Returns
    ///
    /// A receiver that will receive ACP messages for this session.
    pub fn register(&mut self, session_id: &str) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel(CHANNEL_BUFFER_SIZE);
        self.routes.insert(session_id.to_string(), tx);
        rx
    }

    /// Unregister a session, removing its route.
    ///
    /// After unregistering, messages for this session will no longer be routed.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The unique identifier for the session to unregister
    pub fn unregister(&mut self, session_id: &str) {
        self.routes.remove(session_id);
    }

    /// Route an ACP message to the appropriate session.
    ///
    /// Extracts the session ID from the message and sends it to the registered
    /// session's channel.
    ///
    /// # Arguments
    ///
    /// * `msg` - The ACP message to route
    ///
    /// # Returns
    ///
    /// `true` if the message was successfully routed, `false` if no session
    /// was registered for the message's session ID or if the session ID
    /// could not be extracted.
    pub fn route(&self, msg: Value) -> bool {
        let Some(session_id) = Self::extract_session_id(&msg) else {
            return false;
        };

        let Some(sender) = self.routes.get(session_id) else {
            return false;
        };

        // Use try_send to avoid blocking; if buffer is full, message is dropped
        sender.try_send(msg).is_ok()
    }

    /// Extract the session ID from an ACP message.
    ///
    /// ACP messages typically have the session ID in `params.sessionId`.
    ///
    /// # Arguments
    ///
    /// * `msg` - The ACP message to extract from
    ///
    /// # Returns
    ///
    /// The session ID if present, or `None` if not found.
    pub fn extract_session_id(msg: &Value) -> Option<&str> {
        msg.get("params")
            .and_then(|params| params.get("sessionId"))
            .and_then(|id| id.as_str())
    }
}

impl Default for SessionRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_route_to_registered_session() {
        let mut router = SessionRouter::new();
        let session_id = "test-session-123";
        let mut rx = router.register(session_id);

        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "text": "Hello" }
                }
            }
        });

        // Route the message
        let routed = router.route(msg.clone());
        assert!(routed, "Message should be routed successfully");

        // Verify the message was received
        let received = rx.try_recv();
        assert!(received.is_ok(), "Should receive the routed message");
        assert_eq!(received.unwrap(), msg);
    }

    #[tokio::test]
    async fn test_unregistered_session_not_routed() {
        let router = SessionRouter::new();

        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "nonexistent-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk"
                }
            }
        });

        // Try to route to unregistered session
        let routed = router.route(msg);
        assert!(!routed, "Message should not be routed to unregistered session");
    }

    #[tokio::test]
    async fn test_extract_session_id() {
        let msg = json!({
            "method": "session/update",
            "params": {
                "sessionId": "extracted-id-456"
            }
        });

        let session_id = SessionRouter::extract_session_id(&msg);
        assert_eq!(session_id, Some("extracted-id-456"));
    }

    #[tokio::test]
    async fn test_extract_session_id_missing() {
        let msg = json!({
            "method": "some/method",
            "params": {}
        });

        let session_id = SessionRouter::extract_session_id(&msg);
        assert_eq!(session_id, None);
    }

    #[tokio::test]
    async fn test_unregister_stops_routing() {
        let mut router = SessionRouter::new();
        let session_id = "temp-session";
        let _rx = router.register(session_id);

        // Unregister the session
        router.unregister(session_id);

        let msg = json!({
            "params": { "sessionId": session_id }
        });

        let routed = router.route(msg);
        assert!(!routed, "Message should not be routed after unregister");
    }
}

