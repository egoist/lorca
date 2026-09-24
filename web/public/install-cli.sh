#!/bin/sh
# Installs the Lorca CLI on macOS or Linux: https://lorca.app/docs/cli
#
#   curl -fsSL https://lorca.app/install-cli.sh | sh
#
# It downloads the build for this computer from the latest release of github.com/egoist/lorca,
# checks it against the checksum published beside it, puts `lorca` in ~/.local/bin, and adds
# that folder to PATH in your shell's profile. Run it again to update. Settings, as environment
# variables for `sh`:
#
#   LORCA_VERSION=1.0.0      a release to install instead of the latest
#   LORCA_INSTALL_DIR=DIR    where `lorca` goes
#   LORCA_NO_MODIFY_PATH=1   leave shell profiles alone
#
# Everything runs from main, at the end, so a download cut short runs nothing.

set -eu

RELEASES=${LORCA_DOWNLOAD_URL:-https://github.com/egoist/lorca/releases}

say() {
	printf '%s\n' "$*"
}

die() {
	printf 'lorca: %s\n' "$*" >&2
	exit 1
}

# download URL FILE [progress]
download() {
	if command -v curl >/dev/null 2>&1; then
		if [ "${3:-}" = progress ] && [ -t 2 ]; then
			curl -fL --retry 3 --progress-bar -o "$2" "$1"
		else
			curl -fsSL --retry 3 -o "$2" "$1"
		fi
	elif command -v wget >/dev/null 2>&1; then
		wget -q -O "$2" "$1"
	else
		die "curl or wget is needed to download Lorca"
	fi
}

# Sets `target`: the build for this OS and CPU, named as the release names it.
detect_target() {
	os=$(uname -s)
	cpu=$(uname -m)
	case $os in
		Darwin) os=macos ;;
		Linux) os=linux ;;
		MINGW* | MSYS* | CYGWIN*) die "on Windows, install from PowerShell: irm https://lorca.app/install-cli.ps1 | iex" ;;
		*) die "there is no Lorca CLI for $os yet" ;;
	esac
	case $cpu in
		x86_64 | amd64) cpu=x86_64 ;;
		arm64 | aarch64) cpu=aarch64 ;;
		*) die "there is no Lorca CLI for $os on $cpu yet" ;;
	esac
	# There is no build for Intel Macs. A shell under Rosetta says x86_64 on Apple silicon, where
	# the arm64 build runs.
	if [ "$os" = macos ] && [ "$cpu" = x86_64 ]; then
		[ "$(/usr/sbin/sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ] || die "the Lorca CLI needs a Mac with Apple silicon"
		cpu=aarch64
	fi
	target=$os-$cpu
}

# verify FILE: the file against the SHA-256 in FILE.sha256.
verify() {
	expected=$(awk '{ print $1; exit }' "$1.sha256")
	if command -v sha256sum >/dev/null 2>&1; then
		actual=$(sha256sum "$1" | awk '{ print $1 }')
	elif command -v shasum >/dev/null 2>&1; then
		actual=$(shasum -a 256 "$1" | awk '{ print $1 }')
	else
		say "No sha256sum or shasum on this computer: installing without checking the download." >&2
		return 0
	fi
	[ "$actual" = "$expected" ] || die "$(basename "$1") does not match its checksum: the download was damaged, so run the installer again"
}

# add_to_path DIR: a line in the profile of the user's shell that puts DIR on PATH. Sets
# `profile`, and `added` when the line is new.
add_to_path() {
	added=
	# $HOME/... in the profile reads better and follows a home folder that moves.
	case $1 in
		"$HOME"/*) dir="\$HOME/${1#"$HOME"/}" ;;
		*) dir=$1 ;;
	esac
	line="export PATH=\"$dir:\$PATH\""
	case $(basename "${SHELL:-sh}") in
		zsh) profile=${ZDOTDIR:-$HOME}/.zshrc ;;
		bash)
			# Terminals on macOS start login shells, which read the first of these that exists.
			if [ "$(uname -s)" != Darwin ]; then
				profile=$HOME/.bashrc
			elif [ -f "$HOME/.bash_profile" ]; then
				profile=$HOME/.bash_profile
			elif [ -f "$HOME/.bash_login" ]; then
				profile=$HOME/.bash_login
			else
				profile=$HOME/.profile
			fi
			;;
		fish)
			profile=${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/lorca.fish
			line="fish_add_path \"$dir\""
			;;
		*) profile=$HOME/.profile ;;
	esac
	if [ -f "$profile" ] && grep -qF "$line" "$profile"; then
		return 0
	fi
	mkdir -p "$(dirname "$profile")"
	printf '\n# The Lorca CLI\n%s\n' "$line" >>"$profile"
	added=1
}

main() {
	detect_target

	tmp=$(mktemp -d 2>/dev/null || mktemp -d -t lorca)
	trap 'rm -rf "$tmp"' EXIT
	trap 'exit 1' HUP INT TERM

	if [ -n "${LORCA_VERSION:-}" ]; then
		version=${LORCA_VERSION#v}
		case $version in
			'' | *[!0-9.]*) die "not a Lorca version: $LORCA_VERSION" ;;
		esac
		url=$RELEASES/download/cli-v$version
		release="release $version"
	else
		url=$RELEASES/latest/download
		release="the latest release"
	fi

	archive=lorca-cli-$target.tar.gz
	say "Downloading lorca for $target from $release"
	download "$url/$archive" "$tmp/$archive" progress || die "could not download $archive from $release"
	download "$url/$archive.sha256" "$tmp/$archive.sha256" || die "could not download $archive.sha256 from $release"
	verify "$tmp/$archive"
	tar -xzf "$tmp/$archive" -C "$tmp"
	[ -f "$tmp/lorca" ] || die "$archive holds no lorca binary"

	install_dir=${LORCA_INSTALL_DIR:-$HOME/.local/bin}
	mkdir -p "$install_dir" 2>/dev/null || true
	[ -d "$install_dir" ] && [ -w "$install_dir" ] || die "cannot write to $install_dir: set LORCA_INSTALL_DIR to a folder you own"
	# In by rename, not a copy over the old file: a running `lorca serve` keeps the file it
	# started from, and macOS kills a signed binary that changes in place.
	cp "$tmp/lorca" "$install_dir/.lorca-$$"
	chmod 755 "$install_dir/.lorca-$$"
	mv -f "$install_dir/.lorca-$$" "$install_dir/lorca"
	installed=$("$install_dir/lorca" --version 2>/dev/null) || die "$install_dir/lorca does not start on this computer"

	say "Installed $installed at $install_dir/lorca"
	case ":$PATH:" in
		*":$install_dir:"*) ;;
		*)
			if [ "${LORCA_NO_MODIFY_PATH:-}" = 1 ]; then
				say "$install_dir is not on your PATH."
			else
				add_to_path "$install_dir"
				if [ -n "$added" ]; then
					say "Added $install_dir to PATH in $profile. Open a new terminal to use it."
				else
					say "$profile puts $install_dir on PATH. Open a new terminal to use it."
				fi
			fi
			;;
	esac

	say ""
	if [ -f "${LORCA_HOME:-$HOME/.lorca}/machine.json" ]; then
		say "If lorca serve is running, restart it to run the new version."
	else
		say "To make this computer a Runner, pair it with your account and start the service:"
		say "  lorca pair 'lorca://pair?...'   # from Pair a Device in the app"
		say "  lorca serve"
	fi
	say "Docs: https://lorca.app/docs/cli"
}

main "$@"
