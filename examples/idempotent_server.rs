//! Idempotent server: the consumer-side dedup pattern the at-least-once model requires.
//!
//! This crate delivers events **at-least-once**: between `read_from(start)`
//! and `delete_acked(start + count - 1)`, a crash leaves the batch on disk
//! and the next `read_from` returns it AGAIN. The library cannot enforce
//! "exactly once" — duplicate deliveries are a fact of life under crash
//! recovery, transient upload failures, and the cloud-sync drain loop in
//! `examples/cloud_sync.rs`.
//!
//! "Effectively-once" is the SERVER's job. The pattern:
//!
//! 1. Producer tags every event with a stable `(producer_id, seq)` pair.
//!    `producer_id` is a per-producer UUID/name; `seq` is the buffer's
//!    sequence number (returned by `append`).
//! 2. The server keeps a durable record of the highest `seq` it has
//!    ack'd per `producer_id` (typically a `UNIQUE(producer_id)` row in
//!    Postgres, an upsert-key in DynamoDB, etc.).
//! 3. On each "upload", the server upserts the event keyed by
//!    `(producer_id, seq)`. A duplicate upload is a no-op.
//!
//! This example is an in-process simulation of that pattern. It does NOT
//! use segment-buffer — it shows the SERVER-side pattern that consumers
//! of segment-buffer MUST implement for effectively-once delivery.
//!
//! Run with: `cargo run --example idempotent_server`

use std::collections::HashMap;

/// A producer-supplied event. The `(producer_id, seq)` pair is the durable
/// dedup key — both fields MUST be set by the producer before append.
#[derive(Debug, Clone, PartialEq)]
struct Event {
    producer_id: String,
    seq: u64,
    payload: String,
}

/// Idempotent in-process "server". A real server would use Postgres /
/// DynamoDB / etc. with a UNIQUE constraint on (producer_id, seq); this
/// struct mirrors that semantics with a HashMap.
struct IdempotentServer {
    /// Highest ack'd seq per producer. Mirrors `CREATE UNIQUE INDEX on
    /// events(producer_id, seq)` plus a `max(ack_seq) GROUP BY producer_id`
    /// cache. `None` = this producer has never been seen.
    acked: HashMap<String, Option<u64>>,
    /// The full event log — what a real server's events table looks like.
    events: Vec<Event>,
}

impl IdempotentServer {
    fn new() -> Self {
        Self {
            acked: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Upsert an event keyed by `(producer_id, seq)`. Returns `true` if
    /// this call actually inserted the event, `false` if it was a
    /// duplicate of an already-acked event.
    ///
    /// This is the idempotent operation. A cloud upload retry calls this
    /// method with the SAME `(producer_id, seq)` and gets a deterministic
    /// "no-op" outcome.
    fn upsert(&mut self, event: Event) -> bool {
        let already = self.acked.get(&event.producer_id).copied().flatten();
        match already {
            Some(prev) if event.seq <= prev => {
                // Duplicate — already acked in a previous (successful) upload.
                false
            }
            _ => {
                // Accept the event. In a real server this is `INSERT INTO
                // events (producer_id, seq, payload) VALUES (...) ON CONFLICT
                // (producer_id, seq) DO NOTHING` plus an upsert into the
                // acked table.
                self.events.push(event.clone());
                self.acked.insert(event.producer_id, Some(event.seq));
                true
            }
        }
    }

    fn event_count(&self) -> usize {
        self.events.len()
    }
}

fn main() {
    let mut server = IdempotentServer::new();
    let producer = "host-a".to_string();

    // First upload of seqs 0..5 — all accepted.
    for seq in 0..5u64 {
        let event = Event {
            producer_id: producer.clone(),
            seq,
            payload: format!("payload-{seq}"),
        };
        assert!(server.upsert(event));
    }
    assert_eq!(server.event_count(), 5);

    // Simulate the at-least-once failure mode: the producer crashed
    // before seeing the ack for seqs 3 and 4, so it retries them. The
    // server MUST treat them as duplicates (no-op), not as new events.
    for seq in 3..5u64 {
        let event = Event {
            producer_id: producer.clone(),
            seq,
            payload: format!("payload-{seq}"),
        };
        let accepted = server.upsert(event);
        assert!(
            !accepted,
            "duplicate upload of seq {seq} must be a no-op (server-side idempotency)"
        );
    }
    assert_eq!(
        server.event_count(),
        5,
        "duplicate uploads must NOT inflate the event log"
    );

    // New uploads after the crash recovery — accepted as new events.
    for seq in 5..10u64 {
        let event = Event {
            producer_id: producer.clone(),
            seq,
            payload: format!("payload-{seq}"),
        };
        assert!(server.upsert(event));
    }
    assert_eq!(server.event_count(), 10);

    // Multiple producers: each has an independent seq space.
    let producer_b = "host-b".to_string();
    for seq in 0..3u64 {
        let event = Event {
            producer_id: producer_b.clone(),
            seq,
            payload: format!("b-payload-{seq}"),
        };
        assert!(server.upsert(event));
    }
    assert_eq!(server.event_count(), 13);

    println!(
        "Server holds {} events after duplicates and multi-producer traffic.",
        server.event_count()
    );
    println!();
    println!("The contract this demonstrates:");
    println!("  - segment-buffer delivers at-least-once");
    println!("  - duplicate uploads are NORMAL (crash recovery, transient failures)");
    println!("  - idempotency on (producer_id, seq) is the SERVER's job");
    println!("  - a UNIQUE(producer_id, seq) constraint makes the upsert safe");
    println!();
    println!("What segment-buffer does NOT do:");
    println!("  - own a cursor file (REJECTED — see AGENTS.md § Layer split)");
    println!("  - enforce idempotency (no server-side state in a local buffer)");
    println!("  - track which events the cloud has ack'd (the server's concern)");
}
