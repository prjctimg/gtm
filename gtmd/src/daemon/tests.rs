use super::*;

/// A `429` must be told apart from a transient fault, because the startup
/// ladder treats them oppositely: one is retried on a doubling backoff, the
/// other on a long fixed wait, since a spent quota does not recover by trying
/// again sooner.
#[test]
fn a_quota_refusal_is_recognised() {
    // reqwest's rendering, verbatim from the daemon log. This is the form a
    // transport-level refusal takes and the one the log is full of.
    assert!(is_rate_limit(
        "me: http error: status code 429 Too Many Requests"
    ));
    // `ApiError::Regular` displays as `{status}: {message}`, so a typed error
    // arriving through rspotify renders without the "status code" prefix.
    assert!(is_rate_limit("429: rate limited"));
    assert!(is_rate_limit("me: 429: Too Many Requests"));
    // The wrapper `run_sync` adds around whatever rspotify produced.
    assert!(is_rate_limit(
        "spotify playlist sync failed: me: http error: status code 429 Too Many Requests"
    ));
}

/// Everything else keeps the fast ladder.
///
/// A `403` is not a quota problem — it is what a dev-mode app returns for
/// playlist contents — and treating it as one would stall syncs for minutes
/// over a refusal that will never clear.
#[test]
fn other_failures_keep_the_fast_ladder() {
    assert!(!is_rate_limit("me: http error: status code 403 Forbidden"));
    assert!(!is_rate_limit(
        "me: http error: status code 500 Internal Server Error"
    ));
    assert!(!is_rate_limit("spotify not linked"));
    assert!(!is_rate_limit(
        "me: http error: status code 401 Unauthorized"
    ));
    // A longer number must not match: "4291" contains "429" as a substring, and
    // a bare `contains` would call it a quota refusal.
    assert!(
        !is_rate_limit("me: http error: status code 4291 something"),
        "4291 is not 429"
    );
    assert!(!is_rate_limit("me: 1429: rate limited"), "1429 is not 429");
    // A title carrying digits is not a status either.
    assert!(
        !is_rate_limit("me: http error: status code 500 track 429 failed"),
        "429 in a name is not a quota refusal"
    );
}

/// The quota wait has to be shorter than the window Spotify actually hands out.
///
/// A live refusal answered `Retry-After: 17`, so waiting minutes to retry a
/// window measured in seconds means the latch stays set long after the quota is
/// back and the library stays empty while the API is serving. The bound is what
/// keeps that from regressing into the fixed 300s it replaced.
#[test]
fn the_quota_retry_fits_the_window() {
    assert!(
        RATE_LIMIT_RETRY <= 60,
        "a {}s wait outlives the {}-second window spotify reports",
        RATE_LIMIT_RETRY,
        17
    );
    // Still long enough not to spin: a retry costs a real request.
    assert!(
        RATE_LIMIT_RETRY >= 5,
        "retrying in {}s is a spin",
        RATE_LIMIT_RETRY
    );
}

#[test]
fn parse_remote_streams() {
    // Bare HTTP(S) URLs are plain streams.
    assert!(matches!(
        parse_remote_path("https://stream.example.com/radio.mp3"),
        Some(RemoteKind::Stream { .. })
    ));
    // Synthetic podcast paths carry the feed id and episode index.
    assert!(matches!(
        parse_remote_path("podcast://feed-1/2"),
        Some(RemoteKind::Podcast { .. })
    ));
    // Local paths are not remote at all.
    assert!(parse_remote_path("/home/me/song.mp3").is_none());
}
