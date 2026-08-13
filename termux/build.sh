#!/bin/bash
# Termux package build script for termux-create-package

TERMUX_PKG_HOMEPREFIX="@TERMUX_PREFIX@"
TERMUX_PKG_BUILD_IN_SRC=true
TERMUX_PKG_DEPENDS="libpulseaudio"
TERMUX_PKG_RECOMMENDS="pulseaudio"
TERMUX_PKG_DESCRIPTION="Terminal music player with PulseAudio backend"
TERMUX_PKG_MAINTAINER="prjctimg <prjctimg@outlook.com>"
TERMUX_PKG_HOMEPAGE="https://github.com/prjctimg/gtm-rs"
