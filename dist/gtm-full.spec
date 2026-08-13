# gtm-full RPM Spec — binary packaging of prebuilt release artifacts.
#
# Build (from the repository root, with a staged rootfs tree in ./stage):
#   rpmbuild -bb \
#     --define "_topdir $PWD/rpmbuild" \
#     --define "gtm_version <X.Y.Z>" \
#     --define "gtm_staging $PWD/stage" \
#     dist/gtm-full.spec
#
# The stage tree is a rootfs-style layout (usr/bin/gtm, usr/share/man/man1,
# ...) assembled by the release workflow.

%global debug_package %{nil}
%global _userunitdir %{_prefix}/lib/systemd/user

Name: gtm-full
Version: %{gtm_version}
Release: 1
Summary: GTM terminal music player - client and daemon
License: GPL-3.0-only
URL: https://github.com/prjctimg/gtm.rs
Requires: alsa-lib

%description
gtm-full bundles the gtm terminal music player client (TUI + CLI) and the
gtmd background daemon, along with man pages, shell completions, a desktop
entry, and a systemd user service.

%install
cp -a %{gtm_staging}/. %{buildroot}/

%files
%license usr/share/licenses/gtm-full/LICENSE
%{_bindir}/gtm
%{_bindir}/gtmd
%{_userunitdir}/gtmd.service
%{_mandir}/man1/gtm.1*
%{_mandir}/man1/gtmd.1*
%{_mandir}/man1/gtmd-ipc.1*
%{_datadir}/applications/gtm.desktop
%{_datadir}/icons/hicolor/scalable/apps/gtm.svg
%{_datadir}/bash-completion/completions/gtm
%{_datadir}/bash-completion/completions/gtmd
%{_datadir}/zsh/site-functions/_gtm
%{_datadir}/zsh/site-functions/_gtmd
%{_datadir}/fish/vendor_completions.d/gtm.fish
%{_datadir}/fish/vendor_completions.d/gtmd.fish
%{_datadir}/elvish/completions/gtm.elv
%{_datadir}/elvish/completions/gtmd.elv
%{_datadir}/powershell/completions/gtm.ps1
%{_datadir}/powershell/completions/gtmd.ps1

%post
if command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload >/dev/null 2>&1 || true
fi

%preun
if [ "$1" = 0 ] && command -v systemctl >/dev/null 2>&1; then
  systemctl --user disable --now gtmd.service >/dev/null 2>&1 || true
fi

%postun
if command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload >/dev/null 2>&1 || true
fi

%changelog
* %{lua:print(strftime("%a %b %d %Y"))} prjctimg <prjctimg@outlook.com> - %{gtm_version}-1
- Initial binary package release
