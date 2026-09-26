use super::*;


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
