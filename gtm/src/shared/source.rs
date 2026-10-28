// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
//
// This is free software released under the GPL-3.0 license.

//! The playback source vocabulary — where the audio came from.
//!
//! One fact, one string, a fixed set of six. Clients match on the literal, so
//! the set is part of the wire contract and widening it is a protocol change,
//! not a refactor: see `PLAYBACK_SOURCE.md` and `docs/man/gtmd-ipc.1.md`.
//!
//! A plain `&'static str` rather than an enum because this crosses the wire.
//! An enum would serialise to `{"Chart":null}` unless tagged by hand, and the
//! wire form is the contract — what a client in another language sees is a
//! lowercase word. An unknown value is not an error; it is a source this build
//! does not know, and a client renders nothing for it rather than guessing.

/// A file on disk.
pub const SRC_LOCAL: &str = "local";
/// Spotify, via librespot.
pub const SRC_SPOTIFY: &str = "spotify";
/// A Radio Browser station, or any other live stream.
pub const SRC_RADIO: &str = "radio";
/// A podcast episode.
pub const SRC_PODCAST: &str = "podcast";
/// A charts provider's stream.
pub const SRC_CHART: &str = "chart";
/// A catalogue provider's track (Qobuz, Tidal, a media server, ...).
///
/// Distinct from `local` because the bytes are remote and the row may need
/// re-resolution on replay; distinct from `radio` because it has a duration and
/// ends. The client can therefore show the same marker for every external
/// provider without claiming a particular service it cannot know from the path.
pub const SRC_PROVIDER: &str = "provider";

/// Every value in the set, in the order the documentation lists them.
pub const SOURCES: [&str; 6] = [
    SRC_LOCAL,
    SRC_SPOTIFY,
    SRC_RADIO,
    SRC_PODCAST,
    SRC_CHART,
    SRC_PROVIDER,
];

/// Whether `s` is one of the [`SOURCES`].
///
/// The check that keeps the set honest: a caller that maps a source to a label
/// or an icon runs it first, so a typo or a future value degrades to "no
/// marker" instead of rendering a wrong one.
pub fn is_known(s: &str) -> bool {
    SOURCES.contains(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_is_exactly_the_documented_six() {
        assert_eq!(SOURCES.len(), 6);
        assert_eq!(
            SOURCES,
            ["local", "spotify", "radio", "podcast", "chart", "provider"],
            "the wire values are the contract; changing one breaks clients"
        );
    }

    #[test]
    fn every_value_is_lowercase_and_distinct() {
        for s in SOURCES {
            assert_eq!(s, s.to_lowercase(), "{s} is not lowercase");
            assert!(!s.is_empty());
        }
        let mut seen = SOURCES.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), SOURCES.len(), "duplicate source value");
    }

    #[test]
    fn known_values_are_accepted_and_nothing_else_is() {
        for s in SOURCES {
            assert!(is_known(s), "{s} should be known");
        }
        // The empty string is what an old daemon sends, and what a path the
        // daemon could not classify produces. It is "unknown", not "local".
        for s in ["", "LOCAL", "Local", "youtube", "deezer", " spotify"] {
            assert!(!is_known(s), "{s:?} should not be known");
        }
    }
}
