//go:build windows

package main

import (
	"os/exec"
	"syscall"
)

// configureChild gives the CLI a console of its own that never shows. The commands it runs for
// bots share that console; a CLI with none would open a console window for each of them.
func configureChild(command *exec.Cmd) {
	command.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
}

// stopChild ends a CLI the app replaces (a new port). At quit the CLI is left to notice that its
// parent went (`--parent-pid`) and stop its commands itself: Windows has no SIGTERM to ask it.
func stopChild(command *exec.Cmd, restarting bool) {
	if restarting && command.Process != nil {
		_ = command.Process.Kill()
	}
}
