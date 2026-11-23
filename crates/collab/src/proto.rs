//! The y-sync messages (as y-websocket frames them) and the limits client and relay share.
//!
//! One WebSocket binary message carries one or more y-sync messages: `0` sync (step 1 = state
//! vector, step 2 = the update that answers it, update = a change), `1` awareness. The relay adds
//! one message of its own, [`PEERS_TAG`]: how many people are in the room. Clients that do not
//! know it skip it.

use yrs::encoding::read::Cursor;
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::{Decode, Decoder, DecoderV1};
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, StateVector, Transact};

/// Custom y-sync message: the number of people in the room (varint).
pub const PEERS_TAG: u8 = 101;
/// Largest WebSocket message either side accepts.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;
/// Largest room (the artifact's state); the engine keeps the same limit for artifacts.
pub const MAX_ROOM_BYTES: usize = 32 * 1024 * 1024;
/// Largest awareness message the relay passes on.
pub const MAX_AWARENESS: usize = 64 * 1024;
/// Close code: the owner ended the room.
pub const CLOSE_ENDED: u16 = 4404;
/// Close code: the room would grow past its limit.
pub const CLOSE_FULL: u16 = 4413;
/// Close code: the room has as many people as the server allows.
pub const CLOSE_BUSY: u16 = 4429;
/// Close code: a message the relay could not read.
pub const CLOSE_INVALID: u16 = 4400;

/// Room ids: artifact ids, 1 to 64 of `A-Z a-z 0-9 - _`.
pub fn valid_room(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn encode(msg: &Message) -> Vec<u8> {
    msg.encode_v1()
}

/// Every message in one frame; an error when any part of it does not parse.
pub fn decode_all(frame: &[u8]) -> Result<Vec<Message>, String> {
    let mut out = Vec::new();
    let mut rest = frame;
    while !rest.is_empty() {
        let mut decoder = DecoderV1::new(Cursor::new(rest));
        out.push(Message::decode(&mut decoder).map_err(|e| e.to_string())?);
        let left = decoder.read_to_end().map_err(|e| e.to_string())?.len();
        rest = rest.get(rest.len().saturating_sub(left)..).unwrap_or(&[]);
        if out.len() > 1024 {
            return Err("zu viele Nachrichten in einem Paket".into());
        }
    }
    Ok(out)
}

/// Step 1: "this is what I have".
pub fn step1(doc: &yrs::Doc) -> Vec<u8> {
    encode(&Message::Sync(SyncMessage::SyncStep1(doc.transact().state_vector())))
}

/// Step 2: what `doc` has that the holder of `sv` lacks.
pub fn step2(doc: &yrs::Doc, sv: &StateVector) -> Vec<u8> {
    encode(&Message::Sync(SyncMessage::SyncStep2(doc.transact().encode_state_as_update_v1(sv))))
}

pub fn update(update: Vec<u8>) -> Vec<u8> {
    encode(&Message::Sync(SyncMessage::Update(update)))
}

pub fn peers(n: u32) -> Vec<u8> {
    let mut data = Vec::new();
    let mut v = n;
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            data.push(b);
            break;
        }
        data.push(b | 0x80);
    }
    encode(&Message::Custom(PEERS_TAG, data))
}

/// The count in a [`PEERS_TAG`] message.
pub fn read_peers(data: &[u8]) -> Option<u32> {
    let mut n: u32 = 0;
    for (i, b) in data.iter().enumerate().take(5) {
        n |= u32::from(b & 0x7f) << (7 * i as u32);
        if b & 0x80 == 0 {
            return Some(n);
        }
    }
    None
}

/// Apply a v1 update to `doc`; `Ok(Some(delta))` with what was new, `Ok(None)` when nothing was.
pub fn apply(doc: &yrs::Doc, update: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let update = yrs::Update::decode_v1(update).map_err(|e| e.to_string())?;
    let mut txn = doc.try_transact_mut().map_err(|_| "Dokument ist gerade belegt".to_string())?;
    txn.apply_update(update).map_err(|e| e.to_string())?;
    if txn.insert_set().is_empty() && txn.delete_set().is_empty() {
        return Ok(None);
    }
    Ok(Some(txn.encode_update_v1()))
}

/// The whole state of `doc`, as one v1 update.
pub fn full_state(doc: &yrs::Doc) -> Vec<u8> {
    doc.transact().encode_state_as_update_v1(&StateVector::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{GetString, Text};

    #[test]
    fn frames_round_trip() {
        let doc = yrs::Doc::new();
        let text = doc.get_or_insert_text("content");
        text.insert(&mut doc.transact_mut(), 0, "Hallo");
        let mut frame = step1(&doc);
        frame.extend(peers(300));
        let msgs = decode_all(&frame).unwrap();
        assert_eq!(msgs.len(), 2);
        let Message::Custom(PEERS_TAG, data) = &msgs[1] else { panic!("peers") };
        assert_eq!(read_peers(data), Some(300));
        assert!(read_peers(&[0xff; 9]).is_none());
        assert!(decode_all(&[0, 1, 255, 255, 255, 255, 15]).is_err(), "a cut-off message is an error");
        assert!(decode_all(&[0, 7]).is_err());

        // Step 2 answers step 1 with exactly what is missing.
        let other = yrs::Doc::new();
        let sv = other.transact().state_vector();
        let Message::Sync(SyncMessage::SyncStep2(u)) = &decode_all(&step2(&doc, &sv)).unwrap()[0] else { panic!("step2") };
        assert!(apply(&other, u).unwrap().is_some());
        assert!(apply(&other, u).unwrap().is_none(), "nothing new the second time");
        assert_eq!(other.get_or_insert_text("content").get_string(&other.transact()), "Hallo");
        assert!(apply(&other, b"\xff\x00garbage").is_err());
        assert!(valid_room("a-b_C9") && !valid_room("") && !valid_room("../x") && !valid_room(&"a".repeat(65)));
    }
}
