# gtmd RPM Spec
# Build: rpmbuild -ba dist/rpm/gtmd.spec
%global debug_package %{nil}

Name: gtmd
Version: 0.2.7
Release: 1%{?dist}
Summary: gtm background audio daemon
License: GPL-3.0-only
URL: https://github.com/prjctimg/gtm.rs
Source0: %{name}-%{version}.tar.gz
BuildRequires: cargo >= 1.81
BuildRequires: pandoc
%if 0%{?fedora} || 0%{?rhel}
BuildRequires: alsa-lib-devel
%endif
%{?systemd_requires}

%description
gtm is a feature rich and cross platform terminal audio player with
background playback and YouTube/Spotify integration.

%package -n gtm
Summary: gtm client - TUI and CLI for controlling the gtmd daemon
%{?systemd_requires}
Requires: gtmd = %{version}-%{release}

%description -n gtm
gtm is the client for the gtmd music daemon. It provides a full-screen
Terminal User Interface (TUI) with keyboard-driven navigation, and a
command-line interface (CLI) for scripting and headless control.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --workspace

%install
# Binaries
install -Dpm 0755 target/release/gtmd %{buildroot}%{_bindir}/gtmd
install -Dpm 0755 target/release/gtm  %{buildroot}%{_bindir}/gtm

# Systemd user service
install -Dpm 0644 dist/gtmd.service %{buildroot}%{_userunitdir}/gtmd.service

# Man pages (generate from Markdown)
mkdir -p artifacts/man
./scripts/build/manpages.sh artifacts
install -Dpm 0644 artifacts/man/gtmd.1     %{buildroot}%{_mandir}/man1/gtmd.1
install -Dpm 0644 artifacts/man/gtmd-ipc.1 %{buildroot}%{_mandir}/man1/gtmd-ipc.1
install -Dpm 0644 artifacts/man/gtm.1      %{buildroot}%{_mandir}/man1/gtm.1

# Shell completions
cargo run --release --bin release-gen completions artifacts
install -Dpm 0644 artifacts/completions/gtm.bash   %{buildroot}%{_datadir}/bash-completion/completions/gtm
install -Dpm 0644 artifacts/completions/_gtm       %{buildroot}%{_datadir}/zsh/site-functions/_gtm
install -Dpm 0644 artifacts/completions/gtm.fish   %{buildroot}%{_datadir}/fish/vendor_completions.d/gtm.fish
install -Dpm 0644 artifacts/completions/gtm.elv    %{buildroot}%{_datadir}/elvish/lib/gtm.elv
install -Dpm 0644 artifacts/completions/gtm.ps1    %{buildroot}%{_datadir}/powershell/Modules/gtm.ps1
install -Dpm 0644 artifacts/completions/gtmd.bash  %{buildroot}%{_datadir}/bash-completion/completions/gtmd
install -Dpm 0644 artifacts/completions/_gtmd      %{buildroot}%{_datadir}/zsh/site-functions/_gtmd
install -Dpm 0644 artifacts/completions/gtmd.fish  %{buildroot}%{_datadir}/fish/vendor_completions.d/gtmd.fish
install -Dpm 0644 artifacts/completions/gtmd.elv   %{buildroot}%{_datadir}/elvish/lib/gtmd.elv
install -Dpm 0644 artifacts/completions/gtmd.ps1   %{buildroot}%{_datadir}/powershell/Modules/gtmd.ps1

# Desktop file
install -Dpm 0644 dist/gtm.desktop %{buildroot}%{_datadir}/applications/gtm.desktop

%check
cargo test --workspace

%post -n gtm
%systemd_user_post gtmd.service

%preun -n gtm
%systemd_user_preun gtmd.service

%postun -n gtm
%systemd_user_postun gtmd.service

%files -n gtmd
%license LICENSE
%{_bindir}/gtmd
%{_mandir}/man1/gtmd.1*
%{_mandir}/man1/gtmd-ipc.1*
%{_datadir}/bash-completion/completions/gtmd
%{_datadir}/zsh/site-functions/_gtmd
%{_datadir}/fish/vendor_completions.d/gtmd.fish
%{_datadir}/elvish/lib/gtmd.elv
%{_datadir}/powershell/Modules/gtmd.ps1

%files -n gtm
%license LICENSE
%{_bindir}/gtm
%{_userunitdir}/gtmd.service
%{_mandir}/man1/gtm.1*
%{_datadir}/applications/gtm.desktop
%{_datadir}/bash-completion/completions/gtm
%{_datadir}/zsh/site-functions/_gtm
%{_datadir}/fish/vendor_completions.d/gtm.fish
%{_datadir}/elvish/lib/gtm.elv
%{_datadir}/powershell/Modules/gtm.ps1

%changelog
* %{lua:print(strftime("%a %b %d %Y"))} prjctimg <prjctimg@outlook.com> - 0.2.7-1
- feat: config CLI flags, mouse input, real visualizer, Spotify Web search, struct refactors
