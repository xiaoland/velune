using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

// FailureWindow tells why Velune could not start; closing it quits.
sealed class FailureWindow : Window
{
    public FailureWindow(string message)
    {
        Title = "Velune";
        Content = new Grid
        {
            Padding = new Thickness(24),
            Background = Markdown.Brush("SolidBackgroundFillColorBaseBrush"),
            Children =
            {
                new TextBlock
                {
                    Text = message,
                    IsTextSelectionEnabled = true,
                    TextWrapping = TextWrapping.Wrap,
                    VerticalAlignment = VerticalAlignment.Center,
                    HorizontalAlignment = HorizontalAlignment.Center,
                },
            },
        };
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Velune.ico"));
        MainWindow.Size(this, 560, 240, 400, 200);
        Closed += (_, _) => Application.Current.Exit();
    }
}
