package main

import (
	"fmt"
	"strings"
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"rsc.io/qr"
	"velune/bindings"
)

// devicesPane is this PC's side of remote control, and the devices it
// controls, as Mac's device settings are.
type devicesPane struct {
	core *core
	hub  *hub
	win  *mygo.Window
	// open shows a paired device in the main window.
	open       func(device)
	invitation *bindings.BindingRemoteInvitation
	code       *qr.Code
	copied     bool
	invite     string
	busy       bool
	failure    string
}

// hubChanged retires a shown invitation once someone used it, or it expired.
func (p *devicesPane) hubChanged() {
	if p.invitation == nil || p.hub.state == nil {
		return
	}
	for _, pairing := range p.hub.state.IncomingPairings {
		if pairing.Phase == bindings.BindingPairingPhaseAwaitingApproval {
			p.invitation = nil
			return
		}
	}
	if expired(p.invitation.ExpiresAtUnixSeconds) {
		p.invitation = nil
	}
}

// perform runs one call on the remote service and shows its failure; done
// runs on the main thread after work succeeds.
func (p *devicesPane) perform(work func(*bindings.VeluneRemoteEndpoint) error, done func()) {
	if p.busy {
		return
	}
	p.busy, p.failure = true, ""
	run(p.win, func() error {
		endpoint, err := p.core.remote()
		if err != nil {
			return err
		}
		return work(endpoint)
	}, func(err error) {
		p.busy = false
		if err != nil {
			p.failure = describe(err)
			return
		}
		if done != nil {
			done()
		}
	})
}

func (p *devicesPane) createInvitation() {
	var invitation bindings.BindingRemoteInvitation
	p.perform(func(endpoint *bindings.VeluneRemoteEndpoint) (err error) {
		invitation, err = endpoint.CreateInvitation(300)
		return err
	}, func() {
		p.invitation, p.copied = &invitation, false
		p.code, _ = qr.Encode(invitation.Encoded, qr.M)
	})
}

func (p *devicesPane) connect() {
	encoded := strings.TrimSpace(p.invite)
	if encoded == "" {
		return
	}
	if _, err := bindings.DecodeRemoteOffer(encoded); err != nil {
		p.failure = describe(err)
		return
	}
	p.invite = ""
	p.perform(func(endpoint *bindings.VeluneRemoteEndpoint) error {
		_, err := endpoint.BeginPairing(encoded)
		return err
	}, nil)
}

func (p *devicesPane) forget(peer string, pending bool) {
	p.core.forgetConnection(peer)
	p.perform(func(endpoint *bindings.VeluneRemoteEndpoint) error {
		if pending {
			if _, err := endpoint.CancelPairing(peer); err != nil {
				return err
			}
		}
		return endpoint.ForgetDevice(peer)
	}, nil)
}

func (p *devicesPane) view(c *ui.Context) {
	if p.invitation != nil && expired(p.invitation.ExpiresAtUnixSeconds) {
		p.invitation = nil
	}
	p.thisPC(c)
	p.others(c)
	failure := p.failure
	if failure == "" {
		failure = p.hub.failure
	}
	failureText(c, failure)
}

func (p *devicesPane) thisPC(c *ui.Context) {
	state := p.hub.state
	section(c, "这台电脑", func() {
		settingsRow(c, func() {
			ui.Text(c, "允许其他设备控制这台电脑").Grow(1)
			ingress := state != nil && state.ControlIngress
			if ui.Switch(c, &ingress).Label("允许其他设备控制这台电脑").Disabled(state == nil || p.busy).Changed() {
				p.perform(func(endpoint *bindings.VeluneRemoteEndpoint) error {
					return endpoint.SetControlIngress(ingress)
				}, nil)
			}
		})
		settingsRow(c, func() {
			ui.Text(c, "设备名称").Grow(1)
			muted(c, p.hub.deviceName(localDevice))
		})
		if p.invitation != nil {
			p.invitationView(c, *p.invitation)
		} else {
			ui.Row(c).Children(func() {
				if ui.Button(c, "添加控制设备…").Disabled(state == nil || p.busy).Clicked() {
					p.createInvitation()
				}
			})
			muted(c, "在另一台设备上扫描二维码或粘贴邀请，两台设备显示的核对码一致时，在这台电脑上允许。")
		}
		if state == nil {
			return
		}
		for _, grant := range state.IncomingGrants {
			ui.Column(c.Key(grant.PeerId)).Children(func() {
				settingsRow(c, func() {
					ui.Column(c).Grow(1).Gap(2).Children(func() {
						ui.Text(c, grant.Name)
						muted(c, "可以控制这台电脑")
					})
					if ui.Button(c, "撤销").Disabled(p.busy).Clicked() {
						peer := grant.PeerId
						p.perform(func(endpoint *bindings.VeluneRemoteEndpoint) error {
							return endpoint.RevokeControl(peer)
						}, nil)
					}
				})
			})
		}
	})
}

func (p *devicesPane) invitationView(c *ui.Context, invitation bindings.BindingRemoteInvitation) {
	settingsRow(c, func() {
		if p.code != nil {
			qrView(c, p.code, 4)
		}
		ui.Column(c).Grow(1).MinWidth(0).Gap(8).Children(func() {
			ui.Text(c, "在另一台设备上扫描二维码，或复制邀请粘贴到那台设备。")
			muted(c, "到期："+time.Unix(int64(invitation.ExpiresAtUnixSeconds), 0).Format("15:04"))
			ui.Row(c).Gap(8).Children(func() {
				label := "复制邀请"
				if p.copied {
					label = "已复制"
				}
				if ui.Button(c, label).Clicked() {
					c.WriteClipboard(invitation.Encoded)
					p.copied = true
				}
				if ui.Button(c, "完成").Clicked() {
					p.invitation = nil
				}
			})
		})
	}).AlignItems(ui.Start)
}

func (p *devicesPane) others(c *ui.Context) {
	state := p.hub.state
	if state == nil {
		state = &bindings.BindingRemoteServiceSnapshot{}
	}
	section(c, "控制其他设备", func() {
		for _, contact := range state.Contacts {
			_, confirmed := contact.Confirmation.(bindings.BindingContactConfirmationConfirmed)
			status := contactStatus(state, contact.PeerId)
			ui.Column(c.Key(contact.PeerId)).Children(func() {
				settingsRow(c, func() {
					ui.Column(c).Grow(1).Gap(2).Children(func() {
						ui.Text(c, contact.Name)
						muted(c, status)
					})
					if confirmed {
						if ui.Button(c, "打开").Clicked() {
							p.open(device(contact.PeerId))
						}
					} else if ui.Button(c, "取消").Disabled(p.busy).Clicked() {
						p.forget(contact.PeerId, true)
					}
				}).ContextMenu(func(m *ui.Menu) {
					if m.Item("移除此设备").Disabled(p.busy).Chosen() {
						p.forget(contact.PeerId, !confirmed)
					}
				})
			})
		}
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			input := ui.TextInput(c, &p.invite).Placeholder("粘贴另一台设备的邀请").Label("邀请").Grow(1)
			ready := !p.busy && p.hub.state != nil && strings.TrimSpace(p.invite) != ""
			if ui.Button(c, "连接").Disabled(!ready).Clicked() || (input.Submitted() && ready) {
				p.connect()
			}
		})
		muted(c, "在要控制的设备上选择添加控制设备，把它显示的邀请粘贴到这里。连接后核对两台设备的核对码，并在那台设备上允许。")
	})
}

// contactStatus is where pairing with a device stands, or its connection once
// paired, in Mac's words.
func contactStatus(state *bindings.BindingRemoteServiceSnapshot, peer string) string {
	for _, pairing := range state.OutgoingPairings {
		if pairing.PeerId != peer || pairing.Phase == bindings.BindingPairingPhaseGranted {
			continue
		}
		switch pairing.Phase {
		case bindings.BindingPairingPhaseRejected:
			return "对方已拒绝"
		case bindings.BindingPairingPhaseExpired:
			return "邀请已过期"
		case bindings.BindingPairingPhaseDisconnected:
			return "配对时连接中断"
		default:
			code := ""
			if pairing.VerificationCode != nil {
				code = formatCode(*pairing.VerificationCode)
			}
			return "等待对方允许 · 核对码 " + code
		}
	}
	for _, controller := range state.Controllers {
		if controller.PeerId != peer {
			continue
		}
		switch controller.Phase {
		case bindings.BindingControllerPhaseConnected:
			return "已连接"
		case bindings.BindingControllerPhaseConnecting, bindings.BindingControllerPhaseDiscovering:
			return "正在连接"
		case bindings.BindingControllerPhaseRecovering:
			return "正在重新连接"
		case bindings.BindingControllerPhaseBlocked:
			if controller.ErrorCode != nil {
				return fmt.Sprintf("无法连接（%s）", *controller.ErrorCode)
			}
			return "无法连接"
		}
	}
	return "未连接"
}

func expired(expiresAtUnixSeconds uint64) bool {
	return uint64(time.Now().Unix()) > expiresAtUnixSeconds
}

// qrView paints a QR code as dark modules on a light quiet zone.
func qrView(c *ui.Context, code *qr.Code, module float32) {
	const quiet = 4
	side := module * float32(code.Size+quiet*2)
	ui.Column(c).Size(side, side).Shrink(0).Draw(func(p *ui.Painter, r ui.Rect) {
		p.Fill(r, ui.RGB(255, 255, 255), 0)
		for y := range code.Size {
			for x := range code.Size {
				if code.Black(x, y) {
					p.Fill(ui.Rect{X: r.X + module*float32(x+quiet), Y: r.Y + module*float32(y+quiet), W: module, H: module}, ui.RGB(0, 0, 0), 0)
				}
			}
		}
	})
}
