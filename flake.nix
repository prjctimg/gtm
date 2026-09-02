{
  description = "gtm – terminal-based music player daemon and client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
        };
      in
      {
        packages.default = pkgs.stdenv.mkDerivation {
          pname = "gtm";
          version = "0.2.75";

          src = ./.;

          nativeBuildInputs = [ rustToolchain pandoc ];
          buildInputs = with pkgs; [ alsa-lib ];

          buildPhase = ''
            cargo build --release --workspace
          '';

          installPhase = ''
            # Binaries
            install -Dm 0755 target/release/gtmd $out/bin/gtmd
            install -Dm 0755 target/release/gtm  $out/bin/gtm

            # Systemd user service
            install -Dm 0644 dist/gtmd.service $out/lib/systemd/user/gtmd.service

            # Man pages
            mkdir -p artifacts/man
            bash scripts/build/manpages.sh artifacts
            install -Dm 0644 artifacts/man/gtmd.1     $out/share/man/man1/gtmd.1
            install -Dm 0644 artifacts/man/gtmd-ipc.1 $out/share/man/man1/gtmd-ipc.1
            install -Dm 0644 artifacts/man/gtm.1      $out/share/man/man1/gtm.1

            # Shell completions
            cargo run --release --bin release-gen completions artifacts
            install -Dm 0644 artifacts/completions/gtm.bash   $out/share/bash-completion/completions/gtm
            install -Dm 0644 artifacts/completions/_gtm       $out/share/zsh/site-functions/_gtm
            install -Dm 0644 artifacts/completions/gtm.fish   $out/share/fish/vendor_completions.d/gtm.fish
            install -Dm 0644 artifacts/completions/gtmd.bash  $out/share/bash-completion/completions/gtmd
            install -Dm 0644 artifacts/completions/_gtmd      $out/share/zsh/site-functions/_gtmd
            install -Dm 0644 artifacts/completions/gtmd.fish  $out/share/fish/vendor_completions.d/gtmd.fish

            # Desktop file
            install -Dm 0644 dist/gtm.desktop $out/share/applications/gtm.desktop
          '';

          checkPhase = ''
            cargo test --workspace
          '';

          meta = with pkgs.lib; {
            description = "Terminal-based music player daemon and client";
            homepage = "https://github.com/prjctimg/gtm.rs";
            license = licenses.gpl3Only;
            maintainers = [ "prjctimg <prjctimg@outlook.com>" ];
            platforms = platforms.linux;
          };
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [ rustToolchain pkgs.alsa-lib pkgs.pandoc pkgs.cargo-deb ];
        };
      }
    );
}
