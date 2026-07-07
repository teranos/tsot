// Framing at the wasm/env boundary. JS shim writes concatenated
// length-prefixed frames into a single buffer; wasm slices them.

use game::remote_players::{FrameError, parse_frames};

fn frame(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

#[test]
fn empty_buffer_yields_no_frames() {
    let frames = parse_frames(&[]).unwrap();
    assert!(frames.is_empty());
}

#[test]
fn single_frame_round_trips() {
    let buf = frame(b"hello");
    let frames = parse_frames(&buf).unwrap();
    assert_eq!(frames, vec![b"hello".as_slice()]);
}

#[test]
fn two_frames_round_trip() {
    let mut buf = frame(b"one");
    buf.extend_from_slice(&frame(b"two-longer"));
    let frames = parse_frames(&buf).unwrap();
    assert_eq!(frames, vec![b"one".as_slice(), b"two-longer".as_slice()]);
}

#[test]
fn truncated_header_errors() {
    let buf = [0u8, 0u8, 0u8]; // only 3 bytes, need 4 for u32 header
    let err = parse_frames(&buf).expect_err("must fail on short header");
    assert_eq!(err, FrameError::ShortHeader);
}

#[test]
fn truncated_body_errors() {
    let mut buf = 10u32.to_le_bytes().to_vec();
    buf.extend_from_slice(b"only5"); // header says 10, only 5 bytes follow
    let err = parse_frames(&buf).expect_err("must fail on short body");
    assert_eq!(
        err,
        FrameError::ShortBody {
            needed: 10,
            remaining: 5,
        }
    );
}
