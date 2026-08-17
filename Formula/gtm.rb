class Gtm < Formula
  desc "Terminal-based music player daemon and client"
  homepage "https://github.com/prjctimg/gtm.rs"
  license "GPL-3.0-only"
  version "0.2.3"

  on_macos do
    on_arm do
      url "https://github.com/prjctimg/gtm.rs/releases/download/v#{version}/gtm-aarch64-darwin.tar.gz"
      sha256 "PLACEHOLDER_ARM64_SHA256"
    end
    on_intel do
      url "https://github.com/prjctimg/gtm.rs/releases/download/v#{version}/gtm-x86_64-darwin.tar.gz"
      sha256 "PLACEHOLDER_AMD64_SHA256"
    end
  end

  def install
    bin.install "bin/gtm" "bin/gtmd"
    man1.install "man/man1/gtm.1", "man/man1/gtmd.1", "man/man1/gtmd-ipc.1"
    bash_completion.install "completions/gtm.bash" => "gtm"
    zsh_completion.install "completions/_gtm"
    fish_completion.install "completions/gtm.fish" => "gtm.fish"
    elvish_completion.install "completions/gtm.elv"
    powershell_completion.install "completions/gtm.ps1"
  end

  def caveats
    <<~EOS
      Start the daemon:
        gtmd &

      Then use gtm to control playback.

      Systemd user service is also available:
        systemctl --user start gtmd
    EOS
  end

  test do
    system "#{bin}/gtm", "--version"
    system "#{bin}/gtmd", "--version"
  end
end
