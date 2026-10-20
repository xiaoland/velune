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

	newConversationIcon = ui.MustParseSVG(squarePenSVG)
	settingsIcon        = ui.MustParseSVG(settingsSVG)
	addIcon             = ui.MustParseSVG(plusSVG)
	removeIcon          = ui.MustParseSVG(minusSVG)
)

// iconButton is a button showing only an icon, named by label.
func iconButton(c *ui.Context, icon *ui.SVG, label string) ui.Element {
	return ui.Button(c, "").Children(func() { ui.Icon(c, icon) }).Label(label).Tooltip(label)
}
