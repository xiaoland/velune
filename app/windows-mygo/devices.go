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

// devicesPane pairs devices and lists those on either side of control.
type devicesPane struct {
	core       *core
	hub        *hub
	win        *mygo.Window
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

func (p *devicesPane) view(c *ui.Context) {
	if p.invitation != nil && expired(p.invitation.ExpiresAtUnixSeconds) {
		p.invitation = nil
	}
	failure := p.failure
	if failure == "" {
		failure = p.hub.failure
	}
	failureText(c, failure)
	p.thisPC(c)
	p.invitationView(c)
	p.grants(c)
	p.connectView(c)
	p.contacts(c)
}

func (p *devicesPane) thisPC(c *ui.Context) {
	state := p.hub.state
	section(c, "这台电脑", func() {
		settingsRow(c, func() {
			ui.Text(c, "设备名称").Grow(1)
			muted(c, p.hub.deviceName(localDevice))
		})
		settingsRow(c, func() {
			ui.Column(c).Grow(1).Gap(2).Children(func() {
				ui.Text(c, "允许其他设备控制这台电脑")
				muted(c, "关闭后，已配对的设备暂时无法连接。")
			})
			ingress := state != nil && state.ControlIngress
			if ui.Switch(c, &ingress).Disabled(state == nil || p.busy).Changed() {
				p.perform(func(endpoint *bindings.VeluneRemoteEndpoint) error {
					return endpoint.SetControlIngress(ingress)
				}, nil)
			}
		})
	})
}

func (p *devicesPane) invitationView(c *ui.Context) {
	section(c, "添加控制设备", func() {
		muted(c, "在 Android 手机或另一台电脑上扫描二维码或粘贴邀请。对方连接后，这里会请你核对并允许。")
		if p.invitation == nil {
			ui.Row(c).Children(func() {
				if ui.Button(c, "创建邀请").Disabled(p.hub.state == nil || p.busy).Clicked() {
					p.createInvitation()
				}
			})
			return
		}
		invitation := *p.invitation
		ui.Row(c).Gap(16).AlignItems(ui.Start).Children(func() {
			if p.code != nil {
				qrView(c, p.code, 4)
			}
			ui.Column(c).Grow(1).MinWidth(0).Gap(8).Children(func() {
				muted(c, "邀请五分钟内有效，只能使用一次。")
				ui.Text(c, invitation.Encoded).Font(monoFamily()).FontSize(11).Wrap().MaxLines(4).Ellipsis("…").Selectable()
				ui.Row(c).Gap(8).Children(func() {
					label := "复制邀请"
					if p.copied {
						label = "已复制"
					}
					if ui.Button(c, label).Clicked() {
						c.WriteClipboard(invitation.Encoded)
						p.copied = true
					}
					ui.Spacer(c)
					if ui.Button(c, "收回").Clicked() {
						p.invitation = nil
					}
				})
			})
		})
	})
}

func (p *devicesPane) grants(c *ui.Context) {
	var grants []bindings.BindingIncomingControlGrant
	if p.hub.state != nil {
		grants = p.hub.state.IncomingGrants
	}
	section(c, "可以控制这台电脑的设备", func() {
		if len(grants) == 0 {
			muted(c, "没有。")
		}
		for _, grant := range grants {
			ui.Column(c.Key(grant.PeerId)).Children(func() {
				settingsRow(c, func() {
					ui.Text(c, grant.Name).Grow(1)
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

func (p *devicesPane) connectView(c *ui.Context) {
	section(c, "控制另一台设备", func() {
		muted(c, "在另一台电脑的“设置 › 设备”中创建邀请，然后粘贴到这里。")
		ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
			input := ui.TextInput(c, &p.invite).Placeholder("粘贴另一台设备的邀请").Grow(1)
			ready := !p.busy && p.hub.state != nil
			if ui.Button(c, "连接").Disabled(!ready).Clicked() || (input.Submitted() && ready) {
				p.connect()
			}
		})
	})
}

func (p *devicesPane) contacts(c *ui.Context) {
	state := p.hub.state
	if state == nil {
		state = &bindings.BindingRemoteServiceSnapshot{}
	}
	section(c, "这台电脑可以控制的设备", func() {
		if len(state.Contacts) == 0 && len(state.OutgoingPairings) == 0 {
			muted(c, "没有。")
		}
		for _, contact := range state.Contacts {
			_, pending := contact.Confirmation.(bindings.BindingContactConfirmationPending)
			status := contactStatus(state, contact.PeerId, pending)
			ui.Column(c.Key(contact.PeerId)).Children(func() {
				settingsRow(c, func() {
					ui.Column(c).Grow(1).Gap(2).Children(func() {
						ui.Text(c, contact.Name)
						muted(c, status)
					})
					label := "移除"
					if pending {
						label = "取消"
					}
					if ui.Button(c, label).Disabled(p.busy).Clicked() {
						peer := contact.PeerId
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
				})
			})
		}
	})
}

// contactStatus is where pairing with a device stands, or its connection once
// paired.
func contactStatus(state *bindings.BindingRemoteServiceSnapshot, peer string, pending bool) string {
	if pending {
		for _, pairing := range state.OutgoingPairings {
			if pairing.PeerId != peer {
				continue
			}
			switch {
			case pairing.Phase == bindings.BindingPairingPhaseAwaitingApproval && pairing.VerificationCode != nil:
				return fmt.Sprintf("核对码 %s，等待对方允许", formatCode(*pairing.VerificationCode))
			case pairing.Phase == bindings.BindingPairingPhaseRejected:
				return "对方已拒绝"
			case pairing.Phase == bindings.BindingPairingPhaseExpired:
				return "邀请已过期"
			default:
				return "正在连接…"
			}
		}
		return "等待对方允许"
	}
	for _, controller := range state.Controllers {
		if controller.PeerId == peer {
			return phaseName(controller.Phase)
		}
	}
	return "已配对"
}

func phaseName(phase bindings.BindingControllerPhase) string {
	switch phase {
	case bindings.BindingControllerPhaseConnected:
		return "已连接"
	case bindings.BindingControllerPhaseConnecting, bindings.BindingControllerPhaseDiscovering:
		return "正在连接"
	case bindings.BindingControllerPhaseRecovering:
		return "正在恢复连接"
	case bindings.BindingControllerPhaseBlocked:
		return "无法连接"
	default:
		return "未连接"
	}
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
