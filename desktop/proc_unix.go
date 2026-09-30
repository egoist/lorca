//go:build !windows

package main

import (
	"os/exec"
	"syscall"
	"time"
)

func configureChild(command *exec.Cmd) {}

// stopChild sends the CLI SIGTERM, which stops the commands bots left running before it exits,
// and kills it if it is still there five seconds later.
func stopChild(command *exec.Cmd, restarting bool) {
	if command.Process == nil {
		return
	}
	_ = command.Process.Signal(syscall.SIGTERM)
	process := command.Process
	time.AfterFunc(5*time.Second, func() { _ = process.Kill() })
}
