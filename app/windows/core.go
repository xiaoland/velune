package main

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"velune/bindings"
)

// device is the device a view works on: "" for this PC, else a paired
// device's peer ID. The interface is the same for each; only the application
// it calls changes.
type device string

const localDevice device = ""

// core holds this PC's execution core and the remote service opened with it.
// Every call into the bindings blocks, so views run them in goroutines and
// apply the result on the main thread (see run).
type core struct {
	home  string
	local *bindings.VeluneApplication
	// view tells this app's windows apart on a controlled device; one per process.
	view string
	// marks are Velune's mark for light and dark backgrounds.
	marks [2]*ui.SVG

	mu          sync.Mutex
	endpoint    *bindings.VeluneRemoteEndpoint
	connections map[string]*bindings.VeluneApplication
}

func openCore() (*core, error) {
	executable, err := os.Executable()
	if err != nil {
		return nil, err
	}
	directory := filepath.Dir(executable)
	if err := bindings.Load(filepath.Join(directory, libraryName())); err != nil {
		return nil, err
	}
	home, err := applicationHome()
	if err != nil {
		return nil, err
	}
	// Runtime helpers ship in resources beside the executable.
	resources := filepath.Join(directory, "resources")
	if info, err := os.Stat(resources); err != nil || !info.IsDir() {
		return nil, fmt.Errorf("缺少运行时资源目录：%s", resources)
	}
	local, err := bindings.VeluneApplicationOpen(bindings.BindingOptions{HomeDirectory: home, ResourcesDirectory: resources})
	if err != nil {
		return nil, errors.New(describe(err))
	}
	view := make([]byte, 16)
	_, _ = rand.Read(view)
	c := &core{home: home, local: local, view: "windows-" + hex.EncodeToString(view), connections: map[string]*bindings.VeluneApplication{}}
	// The brand's files ship in resources; without them the app shows no mark.
	for i, colorway := range []string{"graphite", "dark"} {
		if data, err := os.ReadFile(filepath.Join(resources, "Brand", "velune-medium-"+colorway+".svg")); err == nil {
			c.marks[i], _ = ui.ParseSVG(data)
		}
	}
	return c, nil
}

// brand is Velune's mark for a light or dark background, or nil.
func (c *core) brand(dark bool) *ui.SVG {
	if dark {
		return c.marks[1]
	}
	return c.marks[0]
}

func libraryName() string {
	switch runtime.GOOS {
	case "windows":
		return "velune_bindings.dll"
	case "darwin":
		return "libvelune_bindings.dylib"
	default:
		return "libvelune_bindings.so"
	}
}

// applicationHome is VELUNE_HOME, else %USERPROFILE%\.velune.
func applicationHome() (string, error) {
	if configured := os.Getenv("VELUNE_HOME"); configured != "" {
		if !filepath.IsAbs(configured) {
			return "", errors.New("VELUNE_HOME 必须使用绝对目录路径；留空可使用默认目录。")
		}
		return configured, nil
	}
	profile, err := os.UserHomeDir()
	if err != nil {
		return "", errors.New("找不到用户目录")
	}
	return filepath.Join(profile, ".velune"), nil
}

func deviceName() string {
	if name := strings.TrimSpace(os.Getenv("COMPUTERNAME")); name != "" {
		return name
	}
	if name, err := os.Hostname(); err == nil && name != "" {
		return name
	}
	return "Windows"
}

// openEndpoint opens the remote service once; incoming control must be
// reachable whenever the app runs.
func (c *core) openEndpoint() (*bindings.VeluneRemoteEndpoint, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.endpoint != nil {
		return c.endpoint, nil
	}
	directory := filepath.Join(c.home, "remote")
	if err := os.MkdirAll(directory, 0o755); err != nil {
		return nil, err
	}
	endpoint, err := bindings.VeluneRemoteEndpointOpen(bindings.BindingRemoteOptions{
		IdentityFile:     filepath.Join(directory, "device-identity"),
		StateFile:        filepath.Join(directory, "remote-state.json"),
		DeviceName:       deviceName(),
		Relay:            bindings.BindingRelayConfigDevelopmentPublic{},
		Discovery:        bindings.BindingDiscoveryConfigDevelopmentPublic{},
		PathPolicy:       bindings.BindingRemotePathPolicyAutomatic,
		RequestTimeoutMs: 30_000,
	}, c.local)
	if err != nil {
		return nil, err
	}
	c.endpoint = endpoint
	return endpoint, nil
}

func (c *core) remote() (*bindings.VeluneRemoteEndpoint, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.endpoint == nil {
		return nil, bindings.BindingErrorUnavailable{}
	}
	return c.endpoint, nil
}

// application is a device's application: this PC's own core, or a paired
// device's through the same named methods.
func (c *core) application(d device) (*bindings.VeluneApplication, error) {
	if d == localDevice {
		return c.local, nil
	}
	c.mu.Lock()
	connection, ok := c.connections[string(d)]
	c.mu.Unlock()
	if ok {
		return connection, nil
	}
	endpoint, err := c.remote()
	if err != nil {
		return nil, err
	}
	connection, err = endpoint.Connect(string(d), c.view)
	if err != nil {
		return nil, err
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if existing, ok := c.connections[string(d)]; ok {
		connection.Destroy()
		return existing, nil
	}
	c.connections[string(d)] = connection
	return connection, nil
}

// forgetConnection drops a removed device's connection; the remote service
// closes it with the device.
func (c *core) forgetConnection(peer string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	delete(c.connections, peer)
}

// shutdown closes connections and the remote service; running conversations
// stay with their owner.
func (c *core) shutdown() {
	c.mu.Lock()
	connections, endpoint := c.connections, c.endpoint
	c.connections = map[string]*bindings.VeluneApplication{}
	c.mu.Unlock()
	for _, connection := range connections {
		_ = connection.Shutdown()
	}
	if endpoint != nil {
		_ = endpoint.Shutdown()
	}
	_ = c.local.Shutdown()
}

// describe is a message the user can act on; details stay in the core's own
// diagnostics.
func describe(err error) string {
	var closed bindings.BindingErrorClosed
	var unavailable bindings.BindingErrorUnavailable
	var diagnostic bindings.BindingErrorDiagnostic
	switch {
	case errors.As(err, &closed):
		return "连接已关闭"
	case errors.As(err, &unavailable):
		return "远程控制服务不可用"
	case errors.As(err, &diagnostic) && diagnostic.OperationId != "":
		return fmt.Sprintf("%s（诊断编号：%s）", diagnostic.Detail, diagnostic.OperationId)
	default:
		return err.Error()
	}
}

// outcome is a blocking call's value or failure, carried to the main thread.
type outcome[T any] struct {
	value T
	err   error
}

// run does blocking work off the main thread, then applies its result there.
func run[T any](win *mygo.Window, work func() T, done func(T)) {
	go func() {
		result := work()
		win.Update(func() { done(result) })
	}()
}
