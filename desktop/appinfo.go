package main

import (
	"os"
	"path/filepath"
	"runtime"

	"github.com/egoist/mygo"
)

// productionRelayURL is the relay a release build's CLI falls back to. Lorca Dev has none; the
// dev loop's relay on this computer stands in.
const productionRelayURL = "https://relay.lorca.app"

// isDevelopment is Lorca Dev: `mygo dev` and `go run` builds, which keep their account and CLI
// apart from the installed Lorca's.
func isDevelopment() bool { return mygo.IsDev() }

// isMock runs the seeded demo instead of the CLI (`LORCA_MOCK=1`), for screenshots.
func isMock() bool { return os.Getenv("LORCA_MOCK") == "1" }

// appName is the name the user sees: "Lorca", or "Lorca Dev" for a development build.
func appName() string {
	if name := mygo.App.Name(); name != "" {
		return name
	}
	if isDevelopment() {
		return "Lorca Dev"
	}
	return "Lorca"
}

func defaultCLIPort() int {
	if isDevelopment() {
		return 4863
	}
	return 4862
}

func defaultCLIHome() string {
	home, _ := os.UserHomeDir()
	if isDevelopment() {
		return filepath.Join(home, ".lorca-dev")
	}
	return filepath.Join(home, ".lorca")
}

// cliCommand is how to start the CLI by hand, for the offline state and the Help note.
func cliCommand() string {
	if isDevelopment() {
		return "lorca serve --home ~/.lorca-dev --port 4863"
	}
	return "lorca serve"
}

// logDir holds cli.log and startup.log.
func logDir() string {
	if dir, err := mygo.App.Path(mygo.PathLogs); err == nil {
		return dir
	}
	return filepath.Join(os.TempDir(), appName())
}

// platformName is the page's name for this system: "windows", "linux", or "darwin".
func platformName() string { return runtime.GOOS }
