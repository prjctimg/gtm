# gtm-mpris

MPRIS D-Bus server for gtm, built on zbus.

Exposes `org.mpris.MediaPlayer2` and `org.mpris.MediaPlayer2.Player` on the session bus so media keys, desktop lock screens, and tools like `playerctl` can control playback. Daemon events are forwarded as `PropertiesChanged` signals.

Enabled by default in `gtmd`; disable with `--no-default-features`.