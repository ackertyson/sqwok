use anyhow::Result;

use crate::channel::protocol::Frame;
use crate::storage::messages::MessageStore;

const MAX_CHUNK_SIZE: usize = 50;

/// Build sync:push frames for an assigned range
pub fn build_sync_responses(
    store: &MessageStore,
    requester: &str,
    from_seq: i64,
    to_seq: i64,
    topic: &str,
) -> Result<Vec<Frame>> {
    let messages = store.get_range(from_seq, to_seq)?;

    let mut frames = Vec::new();
    for chunk in messages.chunks(MAX_CHUNK_SIZE) {
        // Strip local-only fields before transmitting to a peer.
        let stripped: Vec<_> = chunk
            .iter()
            .map(|m| {
                let mut m = m.clone();
                m.as_object_mut().map(|o| o.remove("read"));
                m
            })
            .collect();
        let frame = Frame::new(
            topic,
            "sync:push",
            serde_json::json!({
                "recipient": requester,
                "messages": stripped,
            }),
        );
        frames.push(frame);
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    fn make_store_with_messages(count: i64) -> MessageStore {
        let store = MessageStore::open_in_memory();
        for i in 1..=count {
            store
                .insert_message(&serde_json::json!({
                    "uuid": format!("00000000-0000-0000-0000-{:012}", i),
                    "sender_uuid": "sender",
                    "thread_uuid": null,
                    "reply_to_uuid": null,
                    "global_seq": i,
                    "key_epoch": 0,
                    "ciphertext": B64.encode(format!("msg {i}")),
                    "ts": "2026-03-22T00:00:00Z",
                    "server_ts": "2026-03-22T00:00:01Z",
                }))
                .unwrap();
        }
        store
    }

    #[test]
    fn test_build_sync_responses_single_chunk() {
        let store = make_store_with_messages(10);
        let frames = build_sync_responses(&store, "req-uuid", 1, 10, "chat:test").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].payload["messages"].as_array().unwrap().len(), 10);
    }

    #[test]
    fn test_build_sync_responses_chunks_at_boundary() {
        let store = make_store_with_messages(55);
        let frames = build_sync_responses(&store, "req-uuid", 1, 55, "chat:test").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].payload["messages"].as_array().unwrap().len(), 50);
        assert_eq!(frames[1].payload["messages"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn test_build_sync_responses_empty_range() {
        let store = make_store_with_messages(5);
        let frames = build_sync_responses(&store, "req-uuid", 10, 20, "chat:test").unwrap();
        assert_eq!(frames.len(), 0);
    }
}
