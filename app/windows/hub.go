package main

import (
	"errors"
	"time"

	"github.com/egoist/mygo"
	"velune/bindings"
)

// hub holds the remote service's state, shared by every window. Its fields
// change on the main thread only.
type hub struct {
	core    *core
	state   *bindings.BindingRemoteServiceSnapshot
	failure string
	// changed runs on the main thread after each new state.
	changed func()
}

// start opens the remote service and follows its state until the app quits.
func (h *hub) start(win *mygo.Window) {
	go func() {
		endpoint, err := h.core.openEndpoint()
		var first bindings.BindingRemoteServiceSnapshot
		if err == nil {
			first, err = endpoint.ReadState()
		}
		if err != nil {
			win.Update(func() { h.failure = "远程控制服务无法启动：" + describe(err) })
			return
		}
		h.apply(win, first)
		observer := endpoint.ObserveState()
		for {
			next, err := observer.Next(5_000)
			var closed bindings.BindingErrorClosed
			switch {
			case errors.As(err, &closed):
				return
			case err != nil:
				time.Sleep(time.Second)
			case next != nil:
				h.apply(win, *next)
			}
		}
	}()
}

func (h *hub) apply(win *mygo.Window, state bindings.BindingRemoteServiceSnapshot) {
	win.Update(func() {
		h.state = &state
		if h.changed != nil {
			h.changed()
		}
	})
}

// devices are the paired devices this PC can open: those the issuing device
// has confirmed.
func (h *hub) devices() []bindings.BindingDeviceContact {
	if h.state == nil {
		return nil
	}
	var devices []bindings.BindingDeviceContact
	for _, contact := range h.state.Contacts {
		if _, ok := contact.Confirmation.(bindings.BindingContactConfirmationConfirmed); ok {
			devices = append(devices, contact)
		}
	}
	return devices
}

func (h *hub) deviceName(d device) string {
	if d == localDevice {
		if h.state != nil && h.state.Descriptor != nil {
			return h.state.Descriptor.Name
		}
		return deviceName()
	}
	if h.state != nil {
		for _, contact := range h.state.Contacts {
			if contact.PeerId == string(d) {
				return contact.Name
			}
		}
	}
	return "设备"
}

// setActivity tells the core which device the user looks at, so it keeps
// that connection ready.
func (h *hub) setActivity(previous, current device, foreground bool) {
	endpoint, err := h.core.remote()
	if err != nil {
		return
	}
	go func() {
		if previous != localDevice && previous != current {
			_ = endpoint.SetControllerActivity(string(previous), false, false)
		}
		if current != localDevice {
			_ = endpoint.SetControllerActivity(string(current), true, foreground)
		}
	}()
}
