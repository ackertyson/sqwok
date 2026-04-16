use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

static REF_COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn next_ref() -> String {
    REF_COUNTER.fetch_add(1, Ordering::Relaxed).to_string()
}

/// A Phoenix Channel frame: [join_ref, ref, topic, event, payload]
#[derive(Debug, Clone)]
pub struct Frame {
    pub join_ref: Option<String>,
    pub ref_id: Option<String>,
    pub topic: String,
    pub event: String,
    pub payload: Value,
}

impl Frame {
    pub fn new(topic: &str, event: &str, payload: Value) -> Self {
        let ref_id = REF_COUNTER.fetch_add(1, Ordering::Relaxed).to_string();
        Frame {
            join_ref: None,
            ref_id: Some(ref_id),
            topic: topic.to_string(),
            event: event.to_string(),
            payload,
        }
    }

    pub fn join(topic: &str, payload: Value) -> Self {
        let ref_id = REF_COUNTER.fetch_add(1, Ordering::Relaxed).to_string();
        Frame {
            join_ref: Some(ref_id.clone()),
            ref_id: Some(ref_id),
            topic: topic.to_string(),
            event: "phx_join".to_string(),
            payload,
        }
    }

    pub fn heartbeat() -> Self {
        let ref_id = REF_COUNTER.fetch_add(1, Ordering::Relaxed).to_string();
        Frame {
            join_ref: None,
            ref_id: Some(ref_id),
            topic: "phoenix".to_string(),
            event: "heartbeat".to_string(),
            payload: serde_json::json!({}),
        }
    }

    pub fn encode(&self) -> String {
        let mut map = serde_json::json!({
            "topic": self.topic,
            "event": self.event,
            "ref": self.ref_id,
            "payload": self.payload,
        });
        if let Some(jr) = &self.join_ref {
            map["join_ref"] = Value::String(jr.clone());
        }
        serde_json::to_string(&map).unwrap()
    }

    pub fn decode(text: &str) -> Option<Self> {
        let map: serde_json::Map<String, Value> = serde_json::from_str(text).ok()?;
        Some(Frame {
            join_ref: map
                .get("join_ref")
                .and_then(|v| v.as_str())
                .map(String::from),
            ref_id: map.get("ref").and_then(|v| v.as_str()).map(String::from),
            topic: map.get("topic")?.as_str()?.to_string(),
            event: map.get("event")?.as_str()?.to_string(),
            payload: map.get("payload").cloned().unwrap_or(Value::Null),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let frame = Frame::new("chat:abc", "msg:new", serde_json::json!({"foo": "bar"}));
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();

        assert_eq!(decoded.topic, "chat:abc");
        assert_eq!(decoded.event, "msg:new");
        assert_eq!(decoded.payload["foo"], "bar");
        assert!(decoded.join_ref.is_none());
    }

    #[test]
    fn test_join_frame_has_join_ref() {
        let frame = Frame::join("chat:abc", serde_json::json!({}));
        assert!(frame.join_ref.is_some());
        assert_eq!(frame.join_ref, frame.ref_id);
        assert_eq!(frame.event, "phx_join");
    }

    #[test]
    fn test_heartbeat_frame() {
        let frame = Frame::heartbeat();
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.topic, "phoenix");
        assert_eq!(decoded.event, "heartbeat");
    }

    #[cfg(test)]
    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn frame_roundtrip_preserves_fields(
                topic in "[^\0]{1,64}",
                event in "[^\0]{1,64}",
            ) {
                let frame = Frame::new(&topic, &event, serde_json::json!(null));
                let encoded = frame.encode();
                let decoded = Frame::decode(&encoded).unwrap();
                prop_assert_eq!(decoded.topic, topic);
                prop_assert_eq!(decoded.event, event);
                prop_assert!(decoded.join_ref.is_none());
            }
        }
    }

    #[test]
    fn test_decode_invalid_returns_none() {
        assert!(Frame::decode("not json").is_none());
        assert!(Frame::decode("[1,2,3]").is_none()); // array, not map
    }
}
