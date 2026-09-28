# Homebrew formula for gtm — built from source via cargo (no prebuilt bottles,
# no placeholder SHA hashes). Version is pinned to a git tag; bump `version`
# and `tag` together on each release.
class Gtm < Formula
  desc "Terminal-based music player daemon and client"
  homepage "https://github.com/prjctimg/gtm.rs"
  url "https://github.com/prjctimg/gtm.rs.git",
      tag: "v0.2.83",
      using: :git
  version "0.2.83"
  license "GPL-3.0-only"

  depends_on "rust" => :build
  depends_on "pkg-config" => :build
  depends_on "alsa-lib"

  def install
    system "cargo", "fetch", "--locked"
    system "cargo", "build", "--release", "--locked",
           "--features", "pulseaudio",
           "--package", "gtm"
    bin.install "target/release/gtm"
    bin.install "target/release/gtmd"
  end

  def caveats
    <<~EOS
      gtm does not daemonize. Start the daemon in a terminal of its own:
        gtmd

      Then use gtm to control playback. Logs go to
      $XDG_DATA_HOME/gtm/gtmd.log rather than the terminal.

      A systemd user service is also available:
        systemctl --user enable --now gtmd
    EOS
  end

  test do
    system "#{bin}/gtm", "--version"
    system "#{bin}/gtmd", "--version"
  end
end