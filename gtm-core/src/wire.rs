// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Binary wire protocol (bincode framing) for DaemonEvent streaming
//
// This is free software released under the GPL-3.0 license.

use crate::ipc::DaemonEvent;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct WireFrame {
    pub version: u32,
    pub events: Vec<DaemonEvent>,
}

pub fn encode(events: &[DaemonEvent]) -> Result<Vec<u8>, bincode::Error> {
    let frame = WireFrame {
        version: 1,
        events: events.to_vec(),
    };
    let payload = bincode::serialize(&frame)?;
    let len = payload.len() as u32;
    let mut buf = Vec::with_capacity(4 + len as usize);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

pub fn decode(buf: &[u8]) -> Result<Option<(WireFrame, u128)>, bincode::Error> {
    if buf.len() < 4 {
        return Ok(None);
    }

    let total_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

    if buf.len() < 4 + total_len {
        return Ok(None);
    }
    let frame: WireFrame = bincode::deserialize(&buf[4..4 + total_len])?;
    Ok(Some((frame, 4 + total_len as u128)))
}
