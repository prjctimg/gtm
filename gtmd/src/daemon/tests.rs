use super::*;

/// A `429` must be told apart from a transient fault, because the startup
/// ladder treats them oppositely: one is retried on a doubling backoff, the
/// other on a long fixed wait, since a spent quota does not recover by trying
/// again sooner.
#[test]
fn a_quota_refusal_is_recognised() {
    // The exact wording rspotify renders for `ApiError::Regular { status: 429 }`,
    // as it appears in the daemon log.
    assert!(is_rate_limit(
        "me: http error: status code 429 Too Many Requests"
    ));
    assert!(is_rate_limit("429: rate limited"));
    // A spent quota and nothing cached takes the same long wait, so the string
    // alone decides, not whether a library happens to be on disk.
    assert!(is_rate_limit(
        "spotify playlist sync failed: me: http error: status code 429 Too Many Requests"
    ));
}

/// Everything else keeps the fast ladder. Notably a `403` is not a quota
/// problem — it is what a dev-mode app returns for playlist contents — and
/// treating it as one would stall syncs for minutes over a permanent refusal.
#[test]
fn other_failures_keep_the_fast_ladder() {
    assert!(!is_rate_limit("me: http error: status code 403 Forbidden"));
    assert!(!is_rate_limit(
        "me: http error: status code 500 Internal Server Error"
    ));
    assert!(!is_rate_limit("spotify not linked"));
    // A `429` that appears only inside other text must not be mistaken for the
    // status: a track or playlist name can carry any digits.
    assert!(
        !is_rate_limit("playlist 429 Too Many Requests Later failed to parse"),
        "429 in a name is not a quota refusal"
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
