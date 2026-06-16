use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_BUS_LEN: usize = 10_000;

/// Rare, narrative events. Per-frame events (Tick, StateRead,
/// ViewportRead) used to live here and got emitted ~60×/s for what
/// were really three counters; they're now `AtomicU64`s read on
/// demand via the FFI counter exports. The bus carries only events
/// that warrant a row in the event log.
#[derive(Debug, Clone)]
pub enum TraceEvent {
    Init { spawn_x: f32, spawn_y: f32, spawn_z: i32 },
    Note { tag: &'static str, msg: String },
    Overflow { dropped: u64, since_seq: u64 },
}

impl TraceEvent {
    fn to_json(&self) -> String {
        match self {
            TraceEvent::Init { spawn_x, spawn_y, spawn_z } => format!(
                r#"{{"kind":"Init","spawn_x":{spawn_x},"spawn_y":{spawn_y},"spawn_z":{spawn_z}}}"#
            ),
            TraceEvent::Note { tag, msg } => format!(
                r#"{{"kind":"Note","tag":"{tag}","msg":{}}}"#,
                escape_json_string(msg)
            ),
            TraceEvent::Overflow { dropped, since_seq } => format!(
                r#"{{"kind":"Overflow","dropped":{dropped},"since_seq":{since_seq}}}"#
            ),
        }
    }
}

// ----- per-frame counters (replaces the deleted Tick/StateRead/ViewportRead variants) -----

pub static TICK_COUNT: AtomicU64 = AtomicU64::new(0);
pub static TICK_BLOCKED_COUNT: AtomicU64 = AtomicU64::new(0);
pub static STATE_READ_COUNT: AtomicU64 = AtomicU64::new(0);
pub static VIEWPORT_READ_COUNT: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn count_tick(blocked: bool) {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
    if blocked {
        TICK_BLOCKED_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn count_state_read() {
    STATE_READ_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn count_viewport_read() {
    VIEWPORT_READ_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

static SEQ: AtomicU64 = AtomicU64::new(0);
static OVERFLOW: AtomicU64 = AtomicU64::new(0);
static FIRST_DROP_SEQ: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static BUS: RefCell<VecDeque<(u64, TraceEvent)>> = const { RefCell::new(VecDeque::new()) };
}

pub fn emit(ev: TraceEvent) {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    BUS.with(|b| {
        let mut buf = b.borrow_mut();
        if buf.len() >= MAX_BUS_LEN {
            let before = OVERFLOW.fetch_add(1, Ordering::SeqCst);
            if before == 0 {
                // First drop in this overflow window — record the seq we
                // started dropping from so observers can spot the gap.
                if let Some((oldest_seq, _)) = buf.front() {
                    FIRST_DROP_SEQ.store(*oldest_seq, Ordering::SeqCst);
                }
            }
            buf.pop_front();
        }
        buf.push_back((seq, ev));
    });
}

pub fn drain_json() -> String {
    BUS.with(|b| {
        let mut buf = b.borrow_mut();
        let overflow = OVERFLOW.swap(0, Ordering::SeqCst);
        let first_drop = FIRST_DROP_SEQ.swap(0, Ordering::SeqCst);
        let mut s = String::from("[");
        let mut first = true;
        if overflow > 0 {
            let ev = TraceEvent::Overflow { dropped: overflow, since_seq: first_drop };
            s.push_str(&format!(r#"{{"seq":0,"event":{}}}"#, ev.to_json()));
            first = false;
        }
        for (seq, ev) in buf.iter() {
            if !first { s.push(','); }
            s.push_str(&format!(r#"{{"seq":{seq},"event":{}}}"#, ev.to_json()));
            first = false;
        }
        s.push(']');
        buf.clear();
        s
    })
}

pub fn drain_discard() {
    BUS.with(|b| b.borrow_mut().clear());
}

pub fn pending_count() -> usize {
    BUS.with(|b| b.borrow().len())
}
