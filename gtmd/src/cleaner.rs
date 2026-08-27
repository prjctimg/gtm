// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// YouTube title cleaning and metadata extraction
//
// This is free software released under the GPL-3.0 license.

/// Returns `(artist_option, cleaned_title)`.
pub fn clean_youtube_title(title: &str) -> (Option<String>, String) {
    let mut result = title.to_string();

    // Strip official media tags: (Official Audio), [Official Video], etc.
    let official_tags = [
        "(Official Audio)",
        "(Official Music Video)",
        "(Official Video)",
        "(Official Lyric Video)",
        "(Official Visualizer)",
        "[Official Audio]",
        "[Official Music Video]",
        "[Official Video]",
        "[Official Lyric Video]",
        "[Official Visualizer]",
        "(Lyric Video)",
        "[Lyric Video]",
        "(Audio)",
        "(Video)",
        "(Music Video)",
    ];
    for tag in &official_tags {
        result = result.replace(tag, "");
    }

    // Strip quality tags
    let quality_tags = [
        "[HD]", "[4K]", "[8K]", "[1080p]", "[720p]", "[480p]", "(HD)", "(4K)", "(8K)", "(1080p)",
        "(720p)", "(480p)",
    ];
    for tag in &quality_tags {
        result = result.replace(tag, "");
    }

    // Strip explicit/clean tags
    result = result.replace("(Explicit)", "");
    result = result.replace("(Clean)", "");
    result = result.replace("[Explicit]", "");
    result = result.replace("[Clean]", "");

    // Strip year: (2024), [2024]
    result = strip_bracket_content_matching(&result, |s| {
        s.chars().all(|c| c.is_ascii_digit()) && s.len() == 4
    });

    // Strip generic fillers (longest first so "Official Audio" wins over
    // "Official").
    let mut fillers = [
        "Official",
        "Music",
        "Lyric Video",
        "Audio Only",
        "With Lyrics",
        "Official Lyric Video",
        "Official Music Video",
        "Official Video",
        "Official Audio",
    ];
    fillers.sort_by_key(|f| std::cmp::Reverse(f.len()));
    for filler in &fillers {
        result = result.replace(filler, "");
    }

    // Strip topic channel prefix: "Artist - Topic - Title" → "Title"
    if let Some(pos) = result.find(" - Topic - ") {
        result = result[pos + 11..].to_string();
    } else if let Some(pos) = result.rfind(" - Topic") {
        // "Artist - Topic" prefix only
        if pos < result.len() - 8 {
            // There's content after " - Topic"
            let after = &result[pos + 8..];
            if let Some(rest) = after.strip_prefix(" - ") {
                result = rest.to_string();
            }
        }
    }

    // Strip trailing " - Topic"
    if result.ends_with(" - Topic") {
        result = result[..result.len() - 8].to_string();
    }

    // Strip feature tags: "| feat. Artist", ", ft. Artist", " x Artist" (at end)
    if let Some(pos) = result.rfind('|') {
        let after = &result[pos + 1..];
        let trimmed = after.trim();
        if trimmed.starts_with("feat.") || trimmed.starts_with("ft.") || trimmed.starts_with("x ") {
            result = result[..pos].to_string();
        }
    }
    // Also handle " (feat. Artist)" or " (ft. Artist)" at end
    result = strip_suffix_parenthesized(&result, |s| {
        s.starts_with("feat.") || s.starts_with("ft.") || s.starts_with("x ")
    });

    // Clean up trailing/leading separators
    result = result.trim().to_string();
    while result.ends_with('-') || result.ends_with('|') || result.ends_with(',') {
        result = result
            .trim_end_matches('-')
            .trim_end_matches('|')
            .trim_end_matches(',')
            .trim()
            .to_string();
    }
    while result.starts_with('-') || result.starts_with('|') || result.starts_with(',') {
        result = result
            .trim_start_matches('-')
            .trim_start_matches('|')
            .trim_start_matches(',')
            .trim()
            .to_string();
    }

    // Collapse multiple spaces
    while result.contains("  ") {
        result = result.replace("  ", " ");
    }

    result = result.trim().to_string();

    // Extract artist if title contains " - " separator
    let artist = if let Some(pos) = result.find(" - ") {
        let a = result[..pos].trim().to_string();
        let t = result[pos + 3..].trim().to_string();
        if !a.is_empty() && !t.is_empty() {
            result = t;
            Some(a)
        } else {
            None
        }
    } else {
        None
    };

    // If cleaning produced nothing useful, return original
    if result.is_empty() {
        return (artist, title.to_string());
    }

    (artist, result)
}

/// Strip bracket content `(...)` or `[...]` where the inner content matches a predicate.
fn strip_bracket_content_matching(s: &str, pred: impl Fn(&str) -> bool) -> String {
    let mut result = s.to_string();
    loop {
        let mut found = false;
        // Find last matched bracket pair
        if let Some((_open_ch, _close_ch, open_pos, close_pos)) = find_last_bracket_pair(&result) {
            let inner = &result[open_pos + 1..close_pos];
            if pred(inner) {
                // Remove the bracket and its content, plus any trailing space
                let mut end = close_pos + 1;
                if end < result.len() && result.as_bytes()[end] == b' ' {
                    end += 1;
                }
                result = format!("{}{}", &result[..open_pos], &result[end..]);
                found = true;
            }
        }
        if !found {
            break;
        }
    }
    result
}

/// Strip a suffix parenthesized group matching a predicate.
fn strip_suffix_parenthesized(s: &str, pred: impl Fn(&str) -> bool) -> String {
    if let Some((_open_ch, _close_ch, open_pos, close_pos)) = find_last_bracket_pair(s)
        && close_pos == s.len() - 1
    {
        let inner = &s[open_pos + 1..close_pos];
        if pred(inner) {
            let mut end = open_pos;
            // Also strip preceding space
            while end > 0 && s.as_bytes()[end - 1] == b' ' {
                end -= 1;
            }
            return s[..end].to_string();
        }
    }
    s.to_string()
}

/// Find the last matched bracket pair in a string.
fn find_last_bracket_pair(s: &str) -> Option<(char, char, usize, usize)> {
    let bytes = s.as_bytes();
    let mut last = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' => {
                last = Some((b as char, i));
            }
            b')' => {
                if let Some((open, open_pos)) = last
                    && open == '('
                {
                    return Some(('(', ')', open_pos, i));
                }
                last = None;
            }
            b']' => {
                if let Some((open, open_pos)) = last
                    && open == '['
                {
                    return Some(('[', ']', open_pos, i));
                }
                last = None;
            }
            _ => {}
        }
    }
    None
}

/// Sanitize a string for use as a filesystem path component.
pub fn sanitize_filename(s: &str) -> String {
    let mut result = s.to_string();
    for ch in &['/', '\\', ':', '*', '?', '"', '<', '>', '|'] {
        result = result.replace(*ch, "_");
    }
    // Collapse multiple underscores
    while result.contains("__") {
        result = result.replace("__", "_");
    }
    result.trim().trim_matches('_').to_string()
}

/// Normalize a yt-dlp filename stem into a spaced-out string.
///
/// yt-dlp replaces spaces with underscores in filenames, e.g.
/// `Bazzi_-_Beautiful_feat._Camila_Official_Audio`. Converting the
/// underscores back to spaces lets the existing title cleaning parse the
/// "Artist - Title" structure.
pub fn normalize_filename_stem(stem: &str) -> String {
    let mut result = stem.replace('_', " ");
    while result.contains("  ") {
        result = result.replace("  ", " ");
    }
    result.trim().to_string()
}

/// Clean a filename stem (as produced by yt-dlp) into `(artist, title)`.
pub fn clean_filename_stem(stem: &str) -> (Option<String>, String) {
    clean_youtube_title(&normalize_filename_stem(stem))
}

/// True when the stored track metadata is "unreliable" and would benefit from
/// enrichment: the title is empty, still equals the raw filename stem (nothing
/// could be parsed), or the track lacks an artist or album.
pub fn title_is_unreliable(stem: &str, title: &str, artist: &str, album: &str) -> bool {
    if title.is_empty() || artist.is_empty() || album.is_empty() {
        return true;
    }
    let normalized = normalize_filename_stem(stem);
    title == stem || title == normalized.as_str() || title.eq_ignore_ascii_case(normalized.as_str())
}

/// True when a stored title still looks like a raw filename rather than
/// parsed metadata: it keeps underscores, or matches the raw/normalized stem.
/// Used to decide whether to re-derive the Deezer query from the filename.
pub fn is_filename_like(stem: &str, title: &str) -> bool {
    if title.is_empty() || title.contains('_') {
        return true;
    }
    let normalized = normalize_filename_stem(stem);
    title == stem || title == normalized.as_str() || title.eq_ignore_ascii_case(normalized.as_str())
}

/// Strip control characters so stored titles are clean UTF-8 text.
pub fn sanitize_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_official_audio() {
        let (artist, title) = clean_youtube_title("Drake - God's Plan (Official Audio)");
        assert_eq!(artist.as_deref(), Some("Drake"));
        assert_eq!(title, "God's Plan");
    }

    #[test]
    fn test_strip_multiple_tags() {
        let (_, title) = clean_youtube_title("Song Title (Official Video) [HD] (2024)");
        assert_eq!(title, "Song Title");
    }

    #[test]
    fn test_strip_topic_prefix() {
        let (_, title) = clean_youtube_title("Pink Floyd - Topic - Comfortably Numb");
        assert_eq!(title, "Comfortably Numb");
    }

    #[test]
    fn test_strip_feature_tag() {
        let (_, title) = clean_youtube_title("Song Title | feat. Someone");
        assert_eq!(title, "Song Title");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("Song: Title? Yes!"), "Song_ Title_ Yes!");
    }

    #[test]
    fn test_passthrough_clean_title() {
        let (artist, title) = clean_youtube_title("Some Clean Song Title");
        assert!(artist.is_none());
        assert_eq!(title, "Some Clean Song Title");
    }

    #[test]
    fn test_normalize_filename_stem() {
        assert_eq!(
            normalize_filename_stem("Bazzi_-_Beautiful_feat._Camila_Official_Audio"),
            "Bazzi - Beautiful feat. Camila Official Audio"
        );
        assert_eq!(normalize_filename_stem("No_Spaces"), "No Spaces");
        assert_eq!(normalize_filename_stem("Already Spaced"), "Already Spaced");
    }

    #[test]
    fn test_clean_filename_stem() {
        let (artist, title) = clean_filename_stem("Bazzi_-_Beautiful_feat._Camila_Official_Audio");
        assert_eq!(artist.as_deref(), Some("Bazzi"));
        assert_eq!(title, "Beautiful feat. Camila");
    }

    #[test]
    fn test_title_is_unreliable() {
        assert!(title_is_unreliable(
            "Hello",
            "Hello",
            "Someone",
            "Some Album"
        ));
        assert!(title_is_unreliable("Song", "Song", "", "Album"));
        assert!(title_is_unreliable("Song", "Song", "Artist", ""));
        assert!(!title_is_unreliable(
            "Bazzi_-_Beautiful_feat._Camila_Official_Audio",
            "Beautiful feat. Camila",
            "Bazzi",
            "Cosmic Latte"
        ));
    }

    #[test]
    fn test_sanitize_text() {
        assert_eq!(sanitize_text("Line1\nLine2\r\n"), "Line1Line2");
        assert_eq!(sanitize_text("  padded  "), "padded");
    }

    #[test]
    fn test_is_filename_like() {
        assert!(is_filename_like(
            "Bazzi_-_Beautiful_feat._Camila_Official_Audio",
            "Bazzi_-_Beautiful_feat._Camila__Audio"
        ));
        assert!(is_filename_like(
            "Kygo, Selena Gomez - It Ain't Me (Official Video)",
            "Kygo, Selena Gomez - It Ain't Me (Official Video)"
        ));
        assert!(!is_filename_like(
            "Bazzi_-_Beautiful_feat._Camila_Official_Audio",
            "Beautiful feat. Camila"
        ));
        assert!(!is_filename_like("Some Stem", "Some Stem Cleaned"));
    }
}
