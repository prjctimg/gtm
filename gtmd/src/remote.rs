// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Blocking HTTP byte source for symphonia-based native streaming
//
// This is free software released under the GPL-3.0 license.

use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtm::audio::symphonia::StreamingReopen;
use tracing::warn;

use blowfish::cipher::KeyInit;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Shared slot that receives the latest ICY `StreamTitle` parsed off a live
/// stream. Written on the decode thread; polled at ~1 Hz by the daemon.
pub type IcySlot = Arc<Mutex<Option<String>>>;

/// Shoutcast/ICEcast transparently tag a request with `Icy-MetaData: 1` so it
/// advertises metadata mode.
pub(crate) const ICY_META_HEADER: &str = "Icy-MetaData";
pub(crate) const ICY_META_INTERVAL: &str = "icy-metaint";

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

/// Deezer's 2048-byte Blowfish-CBC per-chunk encryption for full-track MP3s:
/// every third 2048-byte chunk (index 0, 3, 6, ...) of the byte stream is
/// Blowfish-encrypted in CBC mode with the fixed dzr IV, and the partial
/// final chunk (if any) is plaintext. Reading through this wrapper decrypts
/// on the fly so symphonia decodes plain MP3.
///
/// Chunk indices count from the stream start, so a reopened reader (seek /
/// reconnect) must re-wrap the fresh transport with a fresh, empty reader —
/// [`BlowfishReopen`] does exactly that.
pub struct BlowfishReader {
    inner: Box<dyn Read + Send>,
    cipher: blowfish::Blowfish,
    /// Decrypted bytes ready to be served, drained front-to-back.
    pending: Vec<u8>,
    /// Index of the next 2048-byte chunk read from `inner` (0-based).
    chunk: u64,
    eof: bool,
}

/// Size of one Deezer cipher chunk.
const DZR_CHUNK: usize = 2048;
/// Fixed CBC IV used for every encrypted Deezer chunk (dzr/deemix convention).
const DZR_IV: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Build a `Blowfish` instance for a derived Deezer track key (16 bytes is
/// always a valid Blowfish key size).
pub fn blowfish_cipher(key: [u8; 16]) -> blowfish::Blowfish {
    blowfish::Blowfish::new_from_slice(&key).expect("16-byte deezer key")
}

/// Blowfish-CBC decrypt one 2048-byte chunk in place (fixed IV, blocks
/// chained across the chunk).
fn decrypt_deezer_chunk(cipher: &blowfish::Blowfish, chunk: &mut [u8]) {
    use blowfish::cipher::BlockDecrypt;
    use blowfish::cipher::generic_array::GenericArray;
    let mut prev = DZR_IV;
    for i in (0..chunk.len()).step_by(8) {
        let mut ct = [0u8; 8];
        ct.copy_from_slice(&chunk[i..i + 8]);
        let mut blk = GenericArray::clone_from_slice(&ct);
        cipher.decrypt_block(&mut blk);
        for (j, b) in blk.iter().enumerate() {
            chunk[i + j] = b ^ prev[j];
        }
        prev = ct;
    }
}

impl BlowfishReader {
    pub fn new(inner: Box<dyn Read + Send>, cipher: blowfish::Blowfish) -> Self {
        Self {
            inner,
            cipher,
            pending: Vec::new(),
            chunk: 0,
            eof: false,
        }
    }
}

impl Read for BlowfishReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        // Serve any decrypted remainder first; symphonia asks for small
        // buffers, so a single chunk typically fills several reads.
        if !self.pending.is_empty() {
            let n = out.len().min(self.pending.len());
            out[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            return Ok(n);
        }
        if self.eof {
            return Ok(0);
        }
        // Pull one raw 2048-byte chunk from the transport; a short read at
        // EOF is a plaintext tail, so only full chunks are decrypted.
        let mut raw = [0u8; DZR_CHUNK];
        let mut filled = 0;
        while filled < DZR_CHUNK {
            match self.inner.read(&mut raw[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => return Err(e),
            }
        }
        if filled == 0 {
            self.eof = true;
            return Ok(0);
        }
        if self.chunk % 3 == 0 && filled == DZR_CHUNK {
            decrypt_deezer_chunk(&self.cipher, &mut raw);
        }
        self.chunk += 1;
        let n = out.len().min(filled);
        out[..n].copy_from_slice(&raw[..n]);
        if n < filled {
            self.pending.extend_from_slice(&raw[n..filled]);
        }
        Ok(n)
    }
}

/// Re-opener for Deezer's encrypting CDN transport: every reconnect wraps the
/// fresh reader in a fresh decrypting layer so per-chunk alignment (counted
/// from stream start) stays intact.
pub struct BlowfishReopen {
    inner: HttpReopen,
    cipher: blowfish::Blowfish,
}

impl BlowfishReopen {
    pub fn new(url: impl Into<String>, cipher: blowfish::Blowfish) -> Self {
        Self {
            inner: HttpReopen::new(url),
            cipher,
        }
    }
}

impl StreamingReopen for BlowfishReopen {
    fn try_reopen(&self) -> Option<Box<dyn Read + Send>> {
        let inner = self.inner.try_reopen()?;
        Some(Box::new(BlowfishReader::new(inner, self.cipher.clone())))
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

/// Re-opener for live (endless) streams: re-issues the GET against the same
/// URL, re-negotiating ICY metadata mode when the original stream used it.
/// Used both for symphonia-level reconnects and daemon-level restarts after
/// a server-side drop, so a dead radio connection resumes instead of
/// stopping playback.
pub struct LiveReopen {
    url: String,
    want_icy: bool,
    slot: Option<IcySlot>,
}

impl LiveReopen {
    pub fn new(url: impl Into<String>, slot: Option<IcySlot>) -> Self {
        Self {
            url: url.into(),
            want_icy: slot.is_some(),
            slot,
        }
    }
}

impl StreamingReopen for LiveReopen {
    fn try_reopen(&self) -> Option<Box<dyn Read + Send>> {
        let mut req = live_client().get(&self.url);
        if self.want_icy {
            req = req.header(ICY_META_HEADER, "1");
        }
        let resp = req.send().map_err(io_other).ok()?;
        if !resp.status().is_success() {
            return None;
        }
        match (&self.slot, resp.headers().get(ICY_META_INTERVAL)) {
            (Some(slot), Some(v)) => match v.to_str().ok()?.trim().parse::<usize>().ok() {
                Some(n) if n > 0 => {
                    Some(Box::new(IcyReader::new(resp, n, slot.clone())) as Box<dyn Read + Send>)
                }
                _ => Some(Box::new(HttpReader::from_response(resp)) as Box<dyn Read + Send>),
            },
            _ => Some(Box::new(HttpReader::from_response(resp)) as Box<dyn Read + Send>),
        }
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

/// Blocking client for live (endless) streams. Deliberately no total
/// `.timeout()`: reqwest's timeout covers the whole request including body
/// streaming, so applying it here would kill every station ~60s in. Connect
/// setup still fails fast via `CONNECT_TIMEOUT`.
pub fn live_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("reqwest client builder cannot fail")
}

fn io_other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
