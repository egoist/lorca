package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"sync"
	"time"

	"github.com/egoist/mygo"
)

// LaunchFailure is why the CLI is not running, for the offline state to word.
type LaunchFailure struct {
	// Kind is "missing_binary", "launch", "exited", or "startup_closed".
	Kind   string `json:"kind"`
	Binary string `json:"binary,omitempty"`
	Reason string `json:"reason,omitempty"`
	Code   int    `json:"code,omitempty"`
	Log    string `json:"log,omitempty"`
}

// LauncherStatus is where the CLI's lifecycle stands.
type LauncherStatus struct {
	// Kind is "idle", "probing", "starting", "running", or "failed".
	Kind string `json:"kind"`
	// External is a CLI that was already answering on the port: the app uses it and starts none.
	External bool           `json:"external,omitempty"`
	Failure  *LaunchFailure `json:"failure,omitempty"`
}

// launcher makes sure a CLI answers on the port: one already running on this computer, or one it
// starts from the app's resources, logs, and restarts when it exits. The readiness line the CLI
// prints once it listens (`--ready-stdout`) is what the connection waits for.
type launcher struct {
	mu           sync.Mutex
	stopped      bool
	generation   int
	port         int
	status       LauncherStatus
	process      *exec.Cmd
	ready        bool
	probing      bool
	restart      *time.Timer
	restartDelay time.Duration

	onStatus func(LauncherStatus)
	onReady  func()
}

func newLauncher() *launcher {
	return &launcher{status: LauncherStatus{Kind: "idle"}, restartDelay: time.Second}
}

func cliLogPath() string { return filepath.Join(logDir(), "cli.log") }

func (l *launcher) currentStatus() LauncherStatus {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.status
}

// setStatus records a status of the generation that reported it; a stale one is dropped.
func (l *launcher) setStatus(generation int, status LauncherStatus) {
	l.mu.Lock()
	if l.stopped || generation != l.generation {
		l.mu.Unlock()
		return
	}
	l.status = status
	onStatus, onReady := l.onStatus, l.onReady
	l.mu.Unlock()
	if onStatus != nil {
		onStatus(status)
	}
	if status.Kind == "running" && onReady != nil {
		onReady()
	}
}

// ensureRunning looks for the CLI on the port in force and starts one when none answers. A new
// port stops the child of the old one.
func (l *launcher) ensureRunning() {
	port := prefs.cliPort()
	l.mu.Lock()
	if l.stopped {
		l.mu.Unlock()
		return
	}
	if l.restart != nil {
		l.restart.Stop()
		l.restart = nil
	}
	if l.port != port {
		l.stopChildLocked(true)
		l.generation++
		l.probing = false
		l.restartDelay = time.Second
		l.port = port
	}
	generation := l.generation
	if l.process != nil {
		ready := l.ready
		l.mu.Unlock()
		if ready {
			l.setStatus(generation, LauncherStatus{Kind: "running"})
		}
		return
	}
	if l.probing {
		l.mu.Unlock()
		return
	}
	l.probing = true
	l.mu.Unlock()

	startupTrace("CLI probe started")
	l.setStatus(generation, LauncherStatus{Kind: "probing"})
	go func() {
		connection, err := net.DialTimeout("tcp", net.JoinHostPort("127.0.0.1", strconv.Itoa(port)), 1500*time.Millisecond)
		if err == nil {
			connection.Close()
		}
		l.finishProbe(generation, port, err == nil)
	}()
}

func (l *launcher) finishProbe(generation, port int, answered bool) {
	l.mu.Lock()
	if l.stopped || generation != l.generation {
		l.mu.Unlock()
		return
	}
	l.probing = false
	l.mu.Unlock()
	if answered {
		startupTrace("CLI listening")
		l.setStatus(generation, LauncherStatus{Kind: "running", External: true})
		return
	}
	l.spawn(generation, port)
}

func (l *launcher) environment() []string {
	environment := os.Environ()
	set := func(key, value string) {
		if _, ok := os.LookupEnv(key); !ok {
			environment = append(environment, key+"="+value)
		}
	}
	set("RUST_LOG", "lorca=info")
	set("LORCA_HOME", defaultCLIHome())
	if !isDevelopment() {
		set("LORCA_DEFAULT_RELAY_URL", productionRelayURL)
	}
	return environment
}

func (l *launcher) spawn(generation, port int) {
	binary := locateBinary()
	if binary == "" {
		l.setStatus(generation, LauncherStatus{Kind: "failed", Failure: &LaunchFailure{Kind: "missing_binary"}})
		return
	}
	startupTrace("CLI starting")
	l.setStatus(generation, LauncherStatus{Kind: "starting"})

	logPath := cliLogPath()
	_ = os.MkdirAll(filepath.Dir(logPath), 0o755)
	logFile, _ := os.OpenFile(logPath, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)

	command := exec.Command(binary, "serve", "--port", strconv.Itoa(port), "--parent-pid", strconv.Itoa(os.Getpid()), "--ready-stdout")
	command.Env = l.environment()
	if logFile != nil {
		command.Stderr = logFile
	}
	configureChild(command)
	stdout, err := command.StdoutPipe()
	if err == nil {
		err = command.Start()
	}
	if err != nil {
		if logFile != nil {
			logFile.Close()
		}
		l.setStatus(generation, LauncherStatus{Kind: "failed", Failure: &LaunchFailure{Kind: "launch", Binary: binary, Reason: err.Error()}})
		return
	}
	startupTrace("CLI spawned")

	l.mu.Lock()
	stale := l.stopped || generation != l.generation
	if !stale {
		l.process = command
		l.ready = false
	}
	l.mu.Unlock()

	go l.readOutput(generation, port, command, stdout, logFile, logPath)
	go l.wait(generation, command, logFile, logPath)
	// Quitting or a new port raced the start: this child is not wanted.
	if stale {
		stopChild(command, true)
	}
}

// readOutput copies what the CLI prints into its log and waits for its readiness record. A
// startup channel that closes before it means the CLI will not become ready.
func (l *launcher) readOutput(generation, port int, command *exec.Cmd, stdout io.Reader, logFile *os.File, logPath string) {
	reader := bufio.NewReader(stdout)
	ready := false
	for {
		line, err := reader.ReadBytes('\n')
		if len(line) > 0 {
			if logFile != nil {
				_, _ = logFile.Write(line)
			}
			if !ready {
				var record struct {
					Event string `json:"event"`
					Port  int    `json:"port"`
				}
				if json.Unmarshal(line, &record) == nil && record.Event == "ready" && record.Port == port {
					ready = true
					l.mu.Lock()
					current := l.process == command
					if current {
						l.ready = true
						l.restartDelay = time.Second
					}
					l.mu.Unlock()
					if current {
						startupTrace("CLI listening")
						l.setStatus(generation, LauncherStatus{Kind: "running"})
					}
				}
			}
		}
		if err != nil {
			break
		}
	}
	if ready {
		return
	}
	// A CLI that exited closed its output too; `wait` reports that one. One still running a
	// moment later closed its startup channel and will never say it is ready.
	time.Sleep(200 * time.Millisecond)
	l.mu.Lock()
	current := l.process == command && !l.stopped
	l.mu.Unlock()
	if current {
		l.setStatus(generation, LauncherStatus{Kind: "failed", Failure: &LaunchFailure{Kind: "startup_closed", Log: logPath}})
		stopChild(command, true)
	}
}

// wait reports an exit and starts the CLI again after a backoff of one second, doubling to 15.
func (l *launcher) wait(generation int, command *exec.Cmd, logFile *os.File, logPath string) {
	err := command.Wait()
	if logFile != nil {
		logFile.Close()
	}
	code := 0
	var exit *exec.ExitError
	if errors.As(err, &exit) {
		code = exit.ExitCode()
	}
	l.mu.Lock()
	if l.process != command {
		l.mu.Unlock()
		return
	}
	l.process = nil
	l.ready = false
	if l.stopped || generation != l.generation {
		l.mu.Unlock()
		return
	}
	delay := l.restartDelay
	l.restartDelay = min(l.restartDelay*2, 15*time.Second)
	l.restart = time.AfterFunc(delay, func() {
		l.mu.Lock()
		stale := l.stopped || generation != l.generation
		l.mu.Unlock()
		if !stale {
			l.ensureRunning()
		}
	})
	l.mu.Unlock()
	l.setStatus(generation, LauncherStatus{Kind: "failed", Failure: &LaunchFailure{Kind: "exited", Code: code, Log: logPath}})
}

// stop fences further starts and stops the child. At quit a Windows CLI is left to see its
// parent go (`--parent-pid`), which it does within a second, and stop its commands itself.
func (l *launcher) stop() {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.stopped = true
	if l.restart != nil {
		l.restart.Stop()
		l.restart = nil
	}
	l.stopChildLocked(false)
}

func (l *launcher) stopChildLocked(restarting bool) {
	if l.process != nil {
		stopChild(l.process, restarting)
	}
	l.process = nil
	l.ready = false
}

// locateBinary finds the CLI: `LORCA_CLI`, the one in the app's resources, the install script's
// folder, cargo's, then PATH.
func locateBinary() string {
	if override := os.Getenv("LORCA_CLI"); override != "" {
		return override
	}
	name := "lorca"
	if runtime.GOOS == "windows" {
		name = "lorca.exe"
	}
	var candidates []string
	if resources, err := mygo.App.Path(mygo.PathResources); err == nil {
		candidates = append(candidates, filepath.Join(resources, "bin", name))
	}
	if home, err := os.UserHomeDir(); err == nil {
		candidates = append(candidates,
			filepath.Join(home, ".local", "bin", name),
			filepath.Join(home, ".cargo", "bin", name))
	}
	for _, candidate := range candidates {
		if isExecutable(candidate) {
			return candidate
		}
	}
	if found, err := exec.LookPath("lorca"); err == nil {
		return found
	}
	return ""
}

func isExecutable(path string) bool {
	info, err := os.Stat(path)
	if err != nil || info.IsDir() {
		return false
	}
	return runtime.GOOS == "windows" || info.Mode()&0o111 != 0
}

// startupTrace writes launch milestones to startup.log with `LORCA_TRACE_STARTUP=1`.
var startupTrace = func() func(string) {
	if os.Getenv("LORCA_TRACE_STARTUP") != "1" {
		return func(string) {}
	}
	started := time.Now()
	var mu sync.Mutex
	seen := map[string]bool{}
	var file *os.File
	return func(phase string) {
		mu.Lock()
		defer mu.Unlock()
		if seen[phase] {
			return
		}
		seen[phase] = true
		if file == nil {
			_ = os.MkdirAll(logDir(), 0o755)
			file, _ = os.Create(filepath.Join(logDir(), "startup.log"))
		}
		if file != nil {
			fmt.Fprintf(file, "+%.1f ms: %s\n", float64(time.Since(started).Microseconds())/1000, phase)
		}
	}
}()
