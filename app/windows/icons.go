package main

import (
	_ "embed"

	"github.com/egoist/mygo/ui"
)

// Icons from Lucide (icons/LICENSE), drawn in the text color.
var (
	//go:embed icons/square-pen.svg
	squarePenSVG []byte
	//go:embed icons/settings.svg
	settingsSVG []byte
	//go:embed icons/plus.svg
	plusSVG []byte
	//go:embed icons/minus.svg
	minusSVG []byte
	//go:embed icons/circle-alert.svg
	circleAlertSVG []byte
	//go:embed icons/hand.svg
	handSVG []byte

	newConversationIcon = ui.MustParseSVG(squarePenSVG)
	settingsIcon        = ui.MustParseSVG(settingsSVG)
	addIcon             = ui.MustParseSVG(plusSVG)
	removeIcon          = ui.MustParseSVG(minusSVG)
	failedIcon          = ui.MustParseSVG(circleAlertSVG)
	waitingIcon         = ui.MustParseSVG(handSVG)
)

// Marks before a conversation's title: a dot while it is unread, else an
// empty one that keeps the titles in line.
var (
	unreadIcon = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/></svg>`))
	readIcon   = ui.MustParseSVG([]byte(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4" fill-opacity="0"/></svg>`))
)

// iconButton is a button showing only an icon, named by label.
func iconButton(c *ui.Context, icon *ui.SVG, label string) ui.Element {
	return ui.Button(c, "").Children(func() { ui.Icon(c, icon) }).Label(label).Tooltip(label)
}
