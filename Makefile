# Usage:
#   make          – build all binaries (debug)
#   make release  – build all binaries (release)
#   make test     – run all workspace tests
#   make check    – cargo check + cargo clippy
#   make clean    – clean build artifacts
#   make deb      – build .deb packages (requires cargo-deb)
#   make man      – generate man pages (requires pandoc)
#   make completions – generate shell completions
#   make install  – install binaries to $(DESTDIR)$(PREFIX)/bin
#   make rpm      – build RPM (requires rpmbuild)
#   make termux       – cross-compile for Android (aarch64, requires cargo-ndk + NDK)
#   make termux-elf   – strip unsupported ELF sections (requires termux-elf-cleaner)
#   make deb-termux   – build Termux .deb package (requires termux-create-package)

PREFIX ?= /usr/local
exec_prefix ?= $(PREFIX)
datarootdir ?= $(PREFIX)/share
datadir ?= $(datarootdir)
DESTDIR ?=
BINDIR ?= $(exec_prefix)/bin
MANDIR ?= $(datarootdir)/man
SYSTEMD_DIR ?= $(exec_prefix)/lib/systemd/user
APPLICATIONS_DIR ?= $(datadir)/applications
ICONS_DIR ?= $(datadir)/icons/hicolor/scalable/apps
COMPLETIONSDIR ?= $(datadir)/bash-completion/completions
ZSH_COMPLETIONSDIR ?= $(datadir)/zsh/site-functions
FISH_COMPLETIONSDIR ?= $(datadir)/fish/vendor_completions.d
ELVISH_COMPLETIONSDIR ?= $(datadir)/elvish/lib
POWERSHELL_COMPLETIONSDIR ?= $(datadir)/powershell/Modules
ANDROID_API ?= 27

VERSION := $(shell cargo metadata --format-version=1 2>/dev/null | jq -r '.packages[] | select(.name=="gtmd") | .version' 2>/dev/null || echo "0.0.0")

.PHONY: all release test check clean deb man completions install rpm termux termux-elf termux-clean deb-termux

all:
	cargo build

release:
	cargo build --release

test:
	cargo test --workspace

check:
	cargo check --workspace
	cargo clippy --workspace -- -D warnings

clean:
	cargo clean

man:
	./scripts/build/manpages.sh artifacts

completions: release
	cargo run --release --bin release-gen completions artifacts
	cp install.sh artifacts/install.sh

install: release man completions
	install -Dm 0755 target/release/gtmd $(DESTDIR)$(BINDIR)/gtmd
	install -Dm 0755 target/release/gtm  $(DESTDIR)$(BINDIR)/gtm
	install -Dm 0644 artifacts/man/gtmd.1     $(DESTDIR)$(MANDIR)/man1/gtmd.1
	install -Dm 0644 artifacts/man/gtmd-ipc.1 $(DESTDIR)$(MANDIR)/man1/gtmd-ipc.1
	install -Dm 0644 artifacts/man/gtm.1      $(DESTDIR)$(MANDIR)/man1/gtm.1
	install -Dm 0644 artifacts/completions/gtm.bash   $(DESTDIR)$(COMPLETIONSDIR)/gtm
	install -Dm 0644 artifacts/completions/gtmd.bash  $(DESTDIR)$(COMPLETIONSDIR)/gtmd
	install -Dm 0644 artifacts/completions/_gtm        $(DESTDIR)$(ZSH_COMPLETIONSDIR)/_gtm
	install -Dm 0644 artifacts/completions/_gtmd       $(DESTDIR)$(ZSH_COMPLETIONSDIR)/_gtmd
	install -Dm 0644 artifacts/completions/gtm.fish    $(DESTDIR)$(FISH_COMPLETIONSDIR)/gtm.fish
	install -Dm 0644 artifacts/completions/gtmd.fish   $(DESTDIR)$(FISH_COMPLETIONSDIR)/gtmd.fish
	install -Dm 0644 artifacts/completions/gtm.elv     $(DESTDIR)$(ELVISH_COMPLETIONSDIR)/gtm.elv
	install -Dm 0644 artifacts/completions/gtmd.elv    $(DESTDIR)$(ELVISH_COMPLETIONSDIR)/gtmd.elv
	install -Dm 0644 artifacts/completions/gtm.ps1     $(DESTDIR)$(POWERSHELL_COMPLETIONSDIR)/gtm.ps1
	install -Dm 0644 artifacts/completions/gtmd.ps1    $(DESTDIR)$(POWERSHELL_COMPLETIONSDIR)/gtmd.ps1
	install -Dm 0644 dist/gtmd.service $(DESTDIR)$(SYSTEMD_DIR)/gtmd.service

deb: release man completions
	@command -v cargo-deb >/dev/null 2>&1 || { echo "cargo-deb not found. Install with: cargo install cargo-deb"; exit 1; }
	for pkg in gtmd gtm; do \
		mkdir -p "$$pkg/deb-assets/man" "$$pkg/deb-assets/completions"; \
		cp artifacts/man/* "$$pkg/deb-assets/man/"; \
		cp artifacts/completions/* "$$pkg/deb-assets/completions/"; \
	done
	cp target/release/gtmd gtm/deb-assets/gtmd
	cp dist/gtmd.service gtm/deb-assets/
	cp dist/gtmd.service gtmd/deb-assets/
	cargo deb --package gtm
	cargo deb --package gtmd
	for pkg in gtmd gtm; do rm -rf "$$pkg/deb-assets"; done

rpm: release
	@command -v rpmbuild >/dev/null 2>&1 || { echo "rpmbuild not found."; exit 1; }
	tar czf /tmp/gtmd-$(VERSION).tar.gz --transform 's|^|gtmd-$(VERSION)/|' \
		--exclude=target --exclude=.git \
		.
	rpmbuild -tb /tmp/gtmd-$(VERSION).tar.gz

termux:
	@command -v cargo-ndk >/dev/null 2>&1 || { echo "cargo-ndk not found. Install with: cargo install cargo-ndk"; exit 1; }
	CARGO_INCREMENTAL=0 cargo ndk -t arm64-v8a -p $(ANDROID_API) \
		build --release --no-default-features --features pulseaudio

termux-elf:
	@command -v termux-elf-cleaner >/dev/null 2>&1 || \
		{ echo "Install termux-elf-cleaner: pip install termux-elf-cleaner"; exit 1; }
	termux-elf-cleaner target/aarch64-linux-android/release/gtmd
	termux-elf-cleaner target/aarch64-linux-android/release/gtm

termux-release: termux termux-elf

termux-clean:
	rm -rf target/aarch64-linux-android/ target/armv7-linux-androideabi/

deb-termux: termux termux-elf
	@command -v termux-create-package >/dev/null 2>&1 || \
		{ echo "termux-create-package not found. Install with: pip install termux-create-package"; exit 1; }
	./scripts/build/termux-deb.sh
