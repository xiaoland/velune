// Velune for Windows: a full device that runs agents on this PC and controls
// paired devices through the same interface.
package main

import (
	"log"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

func main() {
	core, failure := openCore()
	mygo.App.WhenReady(func() {
		options := mygo.WindowOptions{Title: "Velune", Width: 1180, Height: 780, MinWidth: 720, MinHeight: 480}
		if failure != nil {
			options.Content = ui.View(func(c *ui.Context) {
				ui.Column(c).Fill().Center().Padding(24).Children(func() {
					ui.Text(c, "Velune 无法启动："+failure.Error()).Selectable()
				})
			})
			mygo.NewWindow(options).OnClosed(mygo.App.Quit)
			return
		}
		w := newWorkspace(core, &hub{core: core})
		// The view draws the title bar; the window keeps its buttons.
		options.TitleBarStyle = mygo.TitleBarHidden
		options.Content = ui.View(w.view)
		win := mygo.NewWindow(options)
		w.attach(win)
		// Closing the main window quits; agents on this PC stop with the app.
		win.OnClosed(func() {
			core.shutdown()
			mygo.App.Quit()
		})
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
