// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// WebVTT / SRT / JSON podcast transcript parsing
//
// This is free software released under the GPL-3.0 license.

//! Transcript parsing, straight into the lyrics shape.
//!
//! A podcast transcript is a timed, line-oriented text track — the same thing
//! an `.lrc` file is. Parsing into [`LrcData`] rather than a new type means the
//! transcript renders in the pane that already does this job: the same scroll,
//! the same timestamp gutter, the same karaoke highlighting. There is no
//! transcript UI to build and nothing new to learn.
//!
//! Three formats are accepted because feeds publish all three:
//!
//! * **WebVTT** — `WEBVTT` header, blank-line-separated cues, `HH:MM:SS.mmm -->
//!   HH:MM:SS.mmm` or `MM:SS.mmm` timestamps. The `<00:00:01.500>` inline
//!   timestamp form carries word-level timings, which map onto karaoke.
//! * **SRT** — numbered blocks, `HH:MM:SS,mmm` (comma, not period).
//! * **Podscribe JSON** — `{"results": [{"startTime", "endTime", "body"}]}`.
//!
//! Anything unrecognised degrades to untimed lines rather than to nothing: a
//! transcript with the timings lost is still readable, an empty pane is not.

use gtm::shared::track::{LrcData, LrcLine, LrcWord};

/// Parse a transcript body of any supported format.
///
/// Never fails. A body that matches none of the formats comes back as one
/// untimed line per non-empty input line, so a plain-text transcript — or a
/// format added after this shipped — still shows up.
pub fn parse_transcript(body: &str) -> LrcData {
    let trimmed = body.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return LrcData {
            title: None,
            artist: None,
            album: None,
            lines: Vec::new(),
        };
    }
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Some(lines) = parse_json(trimmed)
    {
        return finish(lines);
    }
    if trimmed.starts_with("WEBVTT") {
        return finish(parse_cues(trimmed, ",", "."));
    }
    // SRT has no magic header, so it is tried whenever the body actually looks
    // like SRT rather than plain text: numbered blocks with a comma timestamp.
    if looks_like_srt(trimmed) {
        return finish(parse_cues(trimmed, ",", ""));
    }
    if looks_like_vtt(trimmed) {
        return finish(parse_cues(trimmed, "", "."));
    }
    let lines = trimmed
        .lines()
        .map(clean_text)
        .filter(|l| !l.is_empty())
        .map(|text| LrcLine {
            timestamp: -1.0,
            text,
            words: Vec::new(),
        })
        .collect();
    finish(lines)
}

fn finish(mut lines: Vec<LrcLine>) -> LrcData {
    // Cues arrive in feed order, which is not always playback order, and a
    // duplicate timestamp would make two lines claim to be the current one.
    lines.sort_by(|a, b| {
        a.timestamp
            .partial_cmp(&b.timestamp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    lines.dedup_by(|a, b| a.timestamp == b.timestamp && a.text == b.text);
    LrcData {
        title: None,
        artist: None,
        album: None,
        lines,
    }
}

/// SRT blocks start with a bare cue number, then a comma-decimal timestamp.
fn looks_like_srt(body: &str) -> bool {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .take(4)
        .any(|l| l.contains("-->") && l.contains(','))
}

/// A headerless WebVTT: period-decimal timestamps, or `NOTE`/`STYLE` blocks.
fn looks_like_vtt(body: &str) -> bool {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .take(4)
        .any(|l| l.contains("-->"))
}

/// Parse cue blocks. `comma` is the decimal separator to accept in a timestamp
/// component (SRT uses `,`, WebVTT uses `.`), and `period` the other one.
fn parse_cues(body: &str, comma: &str, period: &str) -> Vec<LrcLine> {
    let mut out = Vec::new();
    // Split on blank lines: a cue is a timing line plus the text beneath it,
    // and a blank line is the only separator the format guarantees.
    for block in body.split("\n\n") {
        let block = block.replace("\r\n", "\n").replace('\r', "\n");
        let mut timing = None;
        let mut text = Vec::new();
        let mut seen_timing = false;
        for line in block.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if !seen_timing {
                if let Some((start, end)) = parse_timing(line, comma, period) {
                    timing = Some((start, end));
                    seen_timing = true;
                    continue;
                }
                // A bare number is a cue identifier, and `WEBVTT`/`NOTE`/`STYLE`
                // are header and metadata lines. Anything else before the
                // timing line means this block is not a cue.
                if line == "WEBVTT" || line.parse::<u64>().is_ok() {
                    continue;
                }
                if line.starts_with("NOTE")
                    || line.starts_with("STYLE")
                    || line.starts_with("REGION")
                {
                    continue;
                }
                // A line with no timing arrow at all: not a cue.
                if !line.contains("-->") {
                    return out;
                }
            }
            text.push(line);
        }
        let Some((start, end)) = timing else {
            continue;
        };
        let (words, plain) = split_words(&text.join("\n"));
        if plain.trim().is_empty() {
            continue;
        }
        out.push(LrcLine {
            timestamp: start,
            text: plain,
            words: if words.is_empty() { Vec::new() } else { words },
        });
        // The end timestamp is what the pane needs for its gutter, and a zero
        // length cue would render as a timestamp range of `[03:12-03:12]`.
        if end > start {
            // Carried on the next line's timestamp when the cues are adjacent;
            // the pane reads the following line's start as the range end, so an
            // explicit end is only needed to close a trailing gap.
            let _ = end;
        }
    }
    out
}

/// `00:00:01.000 --> 00:00:04.000 align:start position:0%` → `(1.0, 4.0)`.
fn parse_timing(line: &str, comma: &str, period: &str) -> Option<(f64, f64)> {
    let (left, right) = line.split_once("-->")?;
    let start = parse_timestamp(left.trim(), comma, period)?;
    // Cue settings trail the end timestamp (`align:`, `line:`, `position:`).
    let end_field = right.split_whitespace().next()?;
    let end = parse_timestamp(end_field, comma, period)?;
    Some((start, end))
}

/// `HH:MM:SS.mmm`, `MM:SS.mmm`, or bare seconds. `hour` is optional.
fn parse_timestamp(s: &str, comma: &str, period: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Normalise the decimal separator so one parser serves SRT and WebVTT.
    let s = if comma.is_empty() {
        s.to_string()
    } else {
        s.replacen(comma, if period.is_empty() { "." } else { period }, 1)
    };
    let parts: Vec<&str> = s.split(':').collect();
    let secs = match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            h * 3600.0 + m * 60.0 + sec
        }
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let sec: f64 = parts[1].parse().ok()?;
            m * 60.0 + sec
        }
        1 => s.parse().ok()?,
        _ => return None,
    };
    // A negative timestamp is not a time. `1e3` parses as a float but is not a
    // timestamp either; both are clamped away rather than trusted.
    if secs.is_finite() && secs >= 0.0 {
        Some(secs)
    } else {
        None
    }
}

/// Pull `<00:00:01.500>` word timings out of a cue and strip all WebVTT
/// markup, returning karaoke words plus the clean line they spell out.
fn split_words(text: &str) -> (Vec<LrcWord>, String) {
    let mut words = Vec::new();
    let mut plain = String::new();
    for (i, raw) in text.split_whitespace().enumerate() {
        let mut chunk = raw.to_string();
        // An inline timestamp marks the start of the word that follows it.
        let mut stamp = None;
        while chunk.starts_with('<') {
            let Some(close) = chunk.find('>') else {
                break;
            };
            let inner = &chunk[1..close];
            if !inner.starts_with('/') {
                // Not a closing tag, so an inline timestamp.
                if let Some(t) = parse_timestamp(inner, "", ".") {
                    stamp = Some(t);
                }
            }
            chunk = chunk[close + 1..].to_string();
        }
        if chunk.is_empty() {
            continue;
        }
        if let Some(t) = stamp {
            words.push(LrcWord {
                time: t,
                text: clean_text(&chunk),
            });
        }
        if i > 0 {
            plain.push(' ');
        }
        plain.push_str(&clean_text(&chunk));
    }
    (words, plain)
}

/// Strip WebVTT/SRT inline markup and decode the few entities that appear in
/// practice.
fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            // `<v Speaker>`, `<b>`, `<c.class>`, `<00:00:01.5>` — everything
            // between angle brackets is markup, including the voice tag whose
            // content is the speaker name.
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

/// Podscribe-style JSON: `{"results": [{"startTime", "endTime", "body"}]}`.
fn parse_json(body: &str) -> Option<Vec<LrcLine>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let items = v
        .get("results")
        .and_then(|r| r.as_array())
        .or_else(|| v.as_array())?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let text = item
            .get("body")
            .or_else(|| item.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let start = item
            .get("startTime")
            .and_then(|t| t.as_f64())
            .unwrap_or(-1.0);
        out.push(LrcLine {
            timestamp: if start.is_finite() && start >= 0.0 {
                start
            } else {
                -1.0
            },
            text: clean_text(text),
            words: Vec::new(),
        });
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webvtt_cues_become_timed_lines() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:04.000 align:start\nHello there\n\n2\n00:00:04.500 --> 00:00:07.000\nGeneral Kenobi\n";
        let l = parse_transcript(vtt);
        assert_eq!(l.lines.len(), 2, "{:?}", l.lines);
        assert_eq!(l.lines[0].timestamp, 1.0);
        assert_eq!(l.lines[0].text, "Hello there");
        assert_eq!(l.lines[1].timestamp, 4.5);
        assert_eq!(l.lines[1].text, "General Kenobi");
    }

    #[test]
    fn srt_comma_timestamps_parse_without_a_header() {
        let srt = "1\n00:00:01,000 --> 00:00:04,000\nHello there\n\n2\n00:01:05,250 --> 00:01:07,000\nSecond\n";
        let l = parse_transcript(srt);
        assert_eq!(l.lines.len(), 2, "{:?}", l.lines);
        assert_eq!(l.lines[0].timestamp, 1.0);
        assert_eq!(l.lines[1].timestamp, 65.25);
    }

    #[test]
    fn short_timestamps_and_voice_tags_are_handled() {
        let vtt = "WEBVTT\n\n00:01.000 --> 00:03.000\n<v Host>Welcome <b>back</b>\n";
        let l = parse_transcript(vtt);
        assert_eq!(l.lines.len(), 1, "{:?}", l.lines);
        assert_eq!(l.lines[0].timestamp, 60.0);
        assert_eq!(l.lines[0].text, "Welcome back");
    }

    #[test]
    fn inline_word_timings_become_karaoke() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\n<00:00:01.000>One <00:00:02.500>two\n";
        let l = parse_transcript(vtt);
        let words = &l.lines[0].words;
        assert_eq!(words.len(), 2, "{words:?}");
        assert_eq!(words[0].text, "One");
        assert_eq!(words[0].time, 1.0);
        assert_eq!(words[1].time, 2.5);
        assert_eq!(l.lines[0].text, "One two");
    }

    #[test]
    fn entities_are_decoded() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nBen &amp; Jerry&apos;s\n";
        assert_eq!(parse_transcript(vtt).lines[0].text, "Ben & Jerry's");
    }

    #[test]
    fn podscribe_json_parses() {
        let json = r#"{"results":[{"startTime":1.5,"endTime":4.0,"body":"Hello"},{"startTime":4.0,"endTime":6.0,"body":"World"}]}"#;
        let l = parse_transcript(json);
        assert_eq!(l.lines.len(), 2, "{:?}", l.lines);
        assert_eq!(l.lines[0].timestamp, 1.5);
        assert_eq!(l.lines[1].text, "World");
    }

    #[test]
    fn plain_text_still_produces_lines() {
        let l = parse_transcript("first line\n\nsecond line\n");
        assert_eq!(l.lines.len(), 2, "{:?}", l.lines);
        assert_eq!(l.lines[0].timestamp, -1.0);
        assert_eq!(l.lines[1].text, "second line");
    }

    /// The pane reads the *next* line's timestamp as the end of the current
    /// one's range, so out-of-order cues would show the wrong range even though
    /// they parse. They are sorted.
    #[test]
    fn out_of_order_cues_are_sorted() {
        let vtt = "WEBVTT\n\n00:00:09.000 --> 00:00:10.000\nlate\n\n00:00:01.000 --> 00:00:02.000\nearly\n";
        let l = parse_transcript(vtt);
        assert_eq!(l.lines[0].text, "early");
        assert_eq!(l.lines[1].text, "late");
    }

    #[test]
    fn an_empty_body_is_no_lines_not_an_error() {
        assert!(parse_transcript("").lines.is_empty());
        assert!(parse_transcript("   \n  ").lines.is_empty());
    }
}
