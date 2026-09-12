// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Blocking HTTP byte source for symphonia-based native streaming
//
// This is free software released under the GPL-3.0 license.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtm_audio::symphonia::StreamingReopen;
use tracing::warn;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Shared slot that receives the latest ICY `StreamTitle` parsed off a live
/// stream. Written on the decode thread; polled at ~1 Hz by the daemon.
pub type IcySlot = Arc<Mutex<Option<String>>>;

/// Shoutcast/ICEcast transparently tag a request with `Icy-MetaData: 1` so it
/// advertises metadata mode.
pub(crate) const ICY_META_HEADER: &str = "Icy-MetaData";
pub(crate) const ICY_META_INT_HEADER: &str = "icy-metaint";

/// A blocking HTTP(S) transport exposed as a `std::io::Read` so symphonia's
/// decoder can stream radio, podcast, and Subsonic audio natively. Reads block
/// on the decode thread exactly like local file I/O; dropping the source (a
/// pause/stop/switch) closes the underlying connection and cancels the fetch.
pub struct HttpReader {
    response: reqwest::blocking::Response,
}

impl HttpReader {
    /// Issue a GET for `url`, failing fast on non-2xx responses.
    pub fn open(url: &str) -> std::io::Result<Self> {
        let response = build_client()?.get(url).send().map_err(io_other)?;
        if !response.status().is_success() {
            return Err(std::io::Error::other(format!(
                "stream request failed: HTTP {}",
                response.status()
            )));
        }
        Ok(Self { response })
    }

    /// Wrap an already-sent successful response as a read source.
    pub fn from_response(response: reqwest::blocking::Response) -> Self {
        Self { response }
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.response.read(buf)
    }
}

impl Seek for HttpReader {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::other(
            "streaming source does not support seeking",
        ))
    }
}

/// Read wrapper for ICY (Shoutcast/ICEcast) live streams. The body interleaves
/// audio with periodic metadata blocks: every `icy-metaint` bytes of audio a
/// one-byte block-length header (`len * 16` bytes of metadata) follows. This
/// reader strips those blocks while publishing any `StreamTitle='...'` it
/// finds into the shared [`IcySlot`], so the byte stream handed to the decoder
/// remains pure audio.
pub struct IcyReader {
    response: reqwest::blocking::Response,
    metaint: usize,
    /// Audio bytes still to emit before the next metadata block.
    audio_remaining: usize,
    slot: IcySlot,
    /// Single leftover byte held between `read` calls.
    stash: Option<u8>,
}

impl IcyReader {
    /// Consume a successful response under ICY metadata mode, with audio
    /// interleaved every `metaint` bytes.
    pub fn new(response: reqwest::blocking::Response, metaint: usize, slot: IcySlot) -> Self {
        Self {
            response,
            metaint,
            audio_remaining: metaint,
            slot,
            stash: None,
        }
    }

    /// Pull the `StreamTitle='...'` value out of a metadata block body.
    fn capture_title(meta: &[u8]) -> Option<String> {
        let text = String::from_utf8_lossy(meta);
        let key = "StreamTitle='";
        let start = text.find(key)? + key.len();
        let title = text[start..]
            .split('\'')
            .next()
            .unwrap_or_default()
            .trim_matches('\0')
            .trim();
        (!title.is_empty()).then(|| title.to_string())
    }

    /// Read exactly `buf.len()` bytes when make them available; returns the
    /// actual count, which is short at end of stream.
    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut got = 0;
        while got < buf.len() {
            match self.response.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) => return Err(e),
            }
        }
        Ok(got)
    }
}

impl Read for IcyReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut filled = 0;
        while filled < buf.len() {
            if let Some(b) = self.stash.take() {
                buf[filled] = b;
                filled += 1;
                continue;
            }
            if self.audio_remaining > 0 {
                let want = (buf.len() - filled).min(self.audio_remaining);
                let n = self.response.read(&mut buf[filled..filled + want])?;
                if n == 0 {
                    return Ok(filled);
                }
                self.audio_remaining -= n;
                filled += n;
                continue;
            }
            // Audio interval exhausted: consume the metadata block, then reset.
            let mut header = [0u8; 1];
            if self.read_exact(&mut header)? == 0 {
                return Ok(filled);
            }
            let block_len = header[0] as usize * 16;
            if block_len > 0 {
                let mut meta = vec![0u8; block_len];
                self.read_exact(&mut meta)?;
                if let Some(title) = Self::capture_title(&meta) {
                    *self.slot.lock().unwrap() = Some(title);
                }
            }
            self.audio_remaining = self.metaint;
        }
        Ok(filled)
    }
}

/// Re-opens an HTTP transport from the stream start. Used by symphonia to
/// reconnect on seek; sufficient for on-demand content (the decoder then skips
/// forward to the requested offset) and ignored for live transports.
pub struct HttpReopen {
    url: String,
}

impl HttpReopen {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl StreamingReopen for HttpReopen {
    fn try_reopen(&self) -> Option<Box<dyn Read + Send>> {
        HttpReader::open(&self.url)
            .map(|r| Box::new(r) as Box<dyn Read + Send>)
            .map_err(|e| {
                warn!("remote stream reconnect failed: {e}");
                e
            })
            .ok()
    }
}

const USER_AGENT: &str = concat!("gtm/", env!("CARGO_PKG_VERSION"));

/// Shared blocking client with sane timeouts. Requests carry a descriptive
/// User-Agent so streaming providers can identify gtm.
pub fn client() -> reqwest::blocking::Client {
    build_client().expect("reqwest client builder cannot fail")
}

fn build_client() -> std::io::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(READ_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(io_other)
}

fn io_other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
