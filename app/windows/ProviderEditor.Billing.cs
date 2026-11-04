using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Velune;

sealed partial class ProviderEditor
{
    void BuildBilling()
    {
        if (profile != RequestProfile.Native || authenticationSource != ProviderAuthenticationSource.ApiKey ||
            (existing?.ExecutionBackend ?? template?.ExecutionBackend) is not ExecutionBackend.Http)
        {
            return;
        }
        var open = new Button { Content = "用量与账单…" };
        var card = Ui.Card(Ui.TwoLines("组织用量与账单", "保存连接与模型认证后，可另行配置组织查询凭据。报表不代表充值余额。", true), open);
        void Update()
        {
            card.Visibility = VeluneBindingsMethods.ProviderBillingService(endpoint.Text.Trim()) == null ? Visibility.Collapsed : Visibility.Visible;
            open.IsEnabled = existing != null && existing.Endpoint == endpoint.Text.Trim();
        }
        endpoint.TextChanged += (_, _) => Update();
        Loaded += (_, _) => Update();
        Update();
        open.Click += async (_, _) =>
        {
            if (existing is not { } saved || VeluneBindingsMethods.ProviderBillingService(saved.Endpoint) is not { } service)
            {
                return;
            }
            try
            {
                var billing = new ProviderBilling(page.Core.Local, saved.Id, service);
                await page.Window.ShowDialog(billing.Dialog);
            }
            catch (Exception problem)
            {
                Problems.Failing("provider-billing-dialog", "打开用量与账单", problem);
            }
        };
        body.Children.Add(card);
    }
}
