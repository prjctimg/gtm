// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Blocking HTTP byte source for symphonia-based native streaming
//
// This is free software released under the GPL-3.0 license.

use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;

use gtm_audio::symphonia::StreamingReopen;
use tracing::warn;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

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
        let response = build_client()?
            .get(url)
            .send()
            .map_err(io_other)?;
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
        Err(std::io::Error::other("streaming source does not support seeking"))
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