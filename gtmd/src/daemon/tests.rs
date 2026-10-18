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

/// A dead refresh token has to be told apart from a transient fault, and from a
/// quota, because it is the only one of the three worth a browser.
///
/// Nothing classified it before, so a revoked grant surfaced as an opaque
/// `spotify token refresh failed: ...` and sat there.
#[test]
fn a_dead_grant_is_recognised() {
    // What the token endpoint answers a refresh it will not renew with, as
    // rspotify renders it: the OAuth body, with no status in front of it.
    assert!(is_dead_token("spotify token refresh failed: invalid_grant"));
    assert!(is_dead_token(
        "spotify playlist sync failed: me: invalid_grant: Refresh token revoked"
    ));
    // The body sometimes arrives with its own phrasing instead.
    assert!(is_dead_token("Refresh token revoked"));
}

/// A `429` is not a dead grant. The two want opposite things -- one is worth
/// waiting out, the other is worth a browser -- and treating the first as the
/// second would open an authorize page for a quota that recovers on its own.
#[test]
fn a_quota_refusal_is_not_a_dead_grant() {
    assert!(!is_dead_token(
        "me: http error: status code 429 Too Many Requests"
    ));
    assert!(!is_dead_token("spotify not linked"));
    assert!(!is_dead_token(
        "me: http error: status code 500 Internal Server Error"
    ));
}

/// A longer token must not match, for the same reason `4291` is not `429`.
#[test]
fn a_token_in_a_word_is_not_a_dead_grant() {
    assert!(
        !is_dead_token("me: invalid_granted something"),
        "invalid_granted is not invalid_grant"
    );
}

/// The quota wait has to fit the window Spotify actually hands out.
///
/// A live refusal answered `Retry-After: 17`, so waiting minutes to retry a
/// window measured in seconds means the latch stays set long after the quota is
/// back and the library stays empty while the API is serving. These bounds are
/// what keep the wait from drifting back to the fixed 300s it replaced, and
/// they are const so a violation fails the build rather than a run.
const _: () = {
    assert!(
        RATE_LIMIT_RETRY <= 60,
        "the wait outlives the 17-second window spotify reports"
    );
    // Long enough not to spin: every retry costs a real request.
    assert!(RATE_LIMIT_RETRY >= 5, "retrying this fast is a spin");
};

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

/// `chart://` is a stream that a chart handed us, and nothing else.
///
/// Its own arm because a chart preview is the same shape of URL as a pasted
/// station. Both fetch the same way and both are live; only the marker differs,
/// and the marker is the entire reason the prefix exists.
#[test]
fn chart_paths_parse_as_charts_and_keep_the_whole_url() {
    match parse_remote_path("chart://https://audio.example.com/a/b/preview.m4s?token=x&y=2") {
        Some(RemoteKind::Chart { url }) => {
            // The query string is part of the address -- these are signed CDN
            // links, and dropping it would fetch a 403 rather than audio.
            assert_eq!(url, "https://audio.example.com/a/b/preview.m4s?token=x&y=2");
        }
        other => panic!("expected Chart, got {other:?}"),
    }
    // And a chart path is not mistaken for a plain stream.
    assert!(!matches!(
        parse_remote_path("chart://https://x.example.com/s"),
        Some(RemoteKind::Stream { .. })
    ));
}

/// The source a path classifies to.
///
/// The cases are drawn from the paths the daemon actually routes, not from the
/// vocabulary: this is the function that has to tell a Spotify URI from a
/// podcast one from a file, and a test built from the constants would only prove
/// the constants are strings.
#[test]
fn source_follows_the_path_not_the_vocabulary() {
    use gtm::shared::{SRC_CHART, SRC_LOCAL, SRC_PODCAST, SRC_RADIO, SRC_SPOTIFY};

    assert_eq!(source_for_path("spotify:track:abc"), SRC_SPOTIFY);
    assert_eq!(source_for_path("radio://uuid/Station Name"), SRC_RADIO);
    assert_eq!(source_for_path("podcast://feed-1/2"), SRC_PODCAST);
    assert_eq!(
        source_for_path("chart://https://x.example.com/s"),
        SRC_CHART
    );
    // A bare stream is the same evidence a station gives -- no length -- so it
    // is reported as radio rather than guessed at.
    assert_eq!(
        source_for_path("https://stream.example.com/live"),
        SRC_RADIO
    );
    // Everything else is a file.
    assert_eq!(source_for_path("/home/me/song.mp3"), SRC_LOCAL);
    assert_eq!(source_for_path("relative/song.flac"), SRC_LOCAL);
}

/// Every path the daemon routes lands on a value from the documented set.
///
/// The five values are a wire contract that clients match on, so the risk is not
/// a wrong classification but a classification that invents a sixth value. This
/// asserts the set is closed over the inputs that exist.
#[test]
fn every_routed_path_classifies_to_a_known_source() {
    use gtm::shared::is_known;

    for path in [
        "spotify:track:abc",
        "radio://uuid/Name",
        "podcast://feed/1",
        "chart://https://x.example.com/a",
        "https://stream.example.com/live",
        "/home/me/song.mp3",
        "C:\\Music\\song.flac",
        "",
    ] {
        assert!(
            is_known(source_for_path(path)),
            "{path:?} classified to {:?}",
            source_for_path(path)
        );
    }
}

/// The emitter and the parser agree on the prefix.
///
/// Written once each and checked against each other, because they are two ends
/// of one convention in two files: if either side is edited alone, a chart row
/// silently comes back as an ordinary stream and nobody finds out until a client
/// shows "radio" over an Apple chart.
#[test]
fn the_chart_round_trips_through_the_router() {
    for url in [
        "https://audio.example.com/preview.m4s",
        "https://audio.example.com/preview.m4s?token=abc&x=1",
        "http://example.com:8080/a%20b/c.mp3",
    ] {
        let path = chart_uri(url);
        assert!(path.starts_with("chart://"), "{path} lost the prefix");
        match parse_remote_path(&path) {
            Some(RemoteKind::Chart { url: back }) => assert_eq!(back, url),
            other => panic!("{path:?} did not come back as a chart: {other:?}"),
        }
    }
    // A chart row with no playable URL stays blank rather than becoming a path
    // that parses and then fails on a fetch.
    assert_eq!(chart_uri(""), "");
    assert!(parse_remote_path(&chart_uri("")).is_none());
}
