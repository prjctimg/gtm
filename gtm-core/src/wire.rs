// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Binary wire protocol (MessagePack framing) for DaemonEvent streaming
//
// This is free software released under the GPL-3.0 license.

use crate::ipc::DaemonEvent;
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};

pub fn encode(events: &[DaemonEvent]) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    let mut buf = Vec::with_capacity(1024);
    events.serialize(&mut Serializer::new(&mut buf))?;
    let len = buf.len() as u32;
    let mut out = Vec::with_capacity(4 + len);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&buf);
    Ok(out)
}

pub fn decode(buf: &[u8]) -> Result<Option<(Vec<DaemonEvent>, u128)>, rmp_serde::decode::Error> {
    if buf.len() < 4 {
        return Ok(None);
    }

    let total_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

    if buf.len() < 4 + total_len {
        return Ok(None);
    }

    let mut deserializer = Deserializer::new(&buf[4..4 + total_len]);
    let events: Vec<DaemonEvent> = Deserialize::deserialize(&mut deserializer)?;
    Ok(Some((events, (4 + total_len) as u128)))
}