# gtm-audio

Audio backend for `gtm`, behind the `AudioBackend` trait.

The default `SymphoniaBackend` decodes with `symphonia` and outputs through `rodio/cpal` on a dedicated audio thread, tracking playback position and reporting track completion over a channel.

The `pulseaudio` feature (e.g. `gtmd --features pulseaudio`) provides the `PulseAudioMixer` backend instead.

