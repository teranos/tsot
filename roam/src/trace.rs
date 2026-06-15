use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_BUS_LEN: usize = 10_000;

#[derive(Debug, Clone)]
pub enum TraceEvent {
    Init { spawn_x: f32, spawn_y: f32, spawn_z: i32 },
    Tick {
        input_bits: u32,
        dt_ms: f32,
        before_x: f32,
        before_y: f32,
        before_z: i32,
        after_x: f32,
        after_y: f32,
        after_z: i32,
        facing: u8,
        intended_dx: f32,
        intended_dy: f32,
        blocked_x: bool,
        blocked_y: bool,
    },
    StateRead { x: f32, y: f32, z: i32, facing: u8 },
    ViewportRead { view_w: u32, view_h: u32, center_tx: i32, center_ty: i32, z: i32 },
    Note { tag: &'static str, msg: String },
    Overflow { dropped: u64, since_seq: u64 },
}

impl TraceEvent {
    fn to_json(&self) -> String {
        match self {
            TraceEvent::Init { spawn_x, spawn_y, spawn_z } => format!(
                r#"{{"kind":"Init","spawn_x":{spawn_x},"spawn_y":{spawn_y},"spawn_z":{spawn_z}}}"#
            ),
            TraceEvent::Tick {
                input_bits, dt_ms, before_x, before_y, before_z, after_x, after_y, after_z,
                facing, intended_dx, intended_dy, blocked_x, blocked_y,
            } => format!(
                r#"{{"kind":"Tick","input_bits":{input_bits},"dt_ms":{dt_ms},"before_x":{before_x},"before_y":{before_y},"before_z":{before_z},"after_x":{after_x},"after_y":{after_y},"after_z":{after_z},"facing":{facing},"intended_dx":{intended_dx},"intended_dy":{intended_dy},"blocked_x":{blocked_x},"blocked_y":{blocked_y}}}"#
            ),
            TraceEvent::StateRead { x, y, z, facing } => format!(
                r#"{{"kind":"StateRead","x":{x},"y":{y},"z":{z},"facing":{facing}}}"#
            ),
            TraceEvent::ViewportRead { view_w, view_h, center_tx, center_ty, z } => format!(
                r#"{{"kind":"ViewportRead","view_w":{view_w},"view_h":{view_h},"center_tx":{center_tx},"center_ty":{center_ty},"z":{z}}}"#
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
