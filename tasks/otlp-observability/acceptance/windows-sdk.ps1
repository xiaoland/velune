# Runs the actual coordinator and bindings in an isolated console; no product fault switches are added.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..\..')
$root = Join-Path $PWD 'target\telemetry-sdk-acceptance'
New-Item -ItemType Directory -Force $root | Out-Null
$source = [IO.File]::ReadAllText((Join-Path $PWD 'app\windows\ClientTelemetry.cs'))
$source = $source.Replace('    public VeluneTelemetry Binding', '    internal static string? InjectFailure;'+[Environment]::NewLine+'    public VeluneTelemetry Binding')
$source = $source.Replace('maxExportBatchSize: 64)).Build();', 'maxExportBatchSize: 64)).Build(); if (InjectFailure == "traces") throw new InvalidOperationException("synthetic construction fault");')
$source = $source.Replace('exportTimeoutMilliseconds: 3000)).Build();', 'exportTimeoutMilliseconds: 3000)).Build(); if (InjectFailure == "metrics") throw new InvalidOperationException("synthetic construction fault");')
$source = $source.Replace('var response = await base.SendAsync(request, cancellationToken)', 'await Probe.BeforeFirstWrite(cancellationToken); var response = await base.SendAsync(request, cancellationToken)')
if (-not $source.Contains('await Probe.BeforeFirstWrite(cancellationToken)')) { throw 'first-write fixture did not match the actual handler' }
[IO.File]::WriteAllText((Join-Path $root 'ClientTelemetry.cs'), $source)
Copy-Item target\bindings\cs\velune_bindings.cs $root
@'
<Project Sdk="Microsoft.NET.Sdk">
 <PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings><Nullable>enable</Nullable><AllowUnsafeBlocks>true</AllowUnsafeBlocks><LangVersion>latest</LangVersion></PropertyGroup>
 <ItemGroup><PackageReference Include="OpenTelemetry" Version="1.20.1"/><PackageReference Include="OpenTelemetry.Exporter.OpenTelemetryProtocol" Version="1.20.1"/></ItemGroup>
</Project>
'@ | Set-Content (Join-Path $root 'Probe.csproj')
@'
using System.Diagnostics;
using System.Diagnostics.Metrics;
using System.Text.Json;
using Velune.Bindings;
namespace Velune;
static class Probe {
 static volatile bool gate;
 static int writes;
 static TaskCompletionSource entered=new(TaskCreationOptions.RunContinuationsAsynchronously);
 static TaskCompletionSource release=new(TaskCreationOptions.RunContinuationsAsynchronously);
 internal static async Task BeforeFirstWrite(CancellationToken token) {
  if(gate) { entered.TrySetResult(); await release.Task.WaitAsync(token); }
  Interlocked.Increment(ref writes);
 }
 static void Require(bool condition, string message) { if(!condition) throw new InvalidOperationException(message); }
 static async Task<int> Main() {
  try { await Run(); return 0; }
  catch(Exception error) { Console.Error.WriteLine("Telemetry SDK acceptance failed: "+error.Message); return 1; }
 }
 static async Task Run() {
  var home=Path.Combine(AppContext.BaseDirectory,"home-"+Guid.NewGuid().ToString("N"));
  Environment.SetEnvironmentVariable("VELUNE_TELEMETRY_PROFILE",null);
  var run="windows-sdk-"+Guid.NewGuid().ToString("N");
  Environment.SetEnvironmentVariable("VELUNE_RUN_ID",run);
  var owner=new ClientTelemetry(home); owner.Start();
  using var source=new ActivitySource("velune.client.native");
  using var meter=new Meter("velune.client.native");
  var counter=meter.CreateCounter<long>("velune.ui.operations");
  Require(!source.HasListeners()&&!counter.Enabled,"default SDK is active");
  var profile=new BindingTelemetryProfile("http://127.0.0.1:34318",new Dictionary<string,string>(),1);
  foreach(var stage in new[]{"traces","metrics"}) {
   ClientTelemetry.InjectFailure=stage;
   bool failed=false;
   try { owner.Save(owner.Binding.Snapshot().Revision,BindingTelemetryPreference.On,profile); }
   catch(InvalidOperationException e) when(e.Message=="synthetic construction fault") { failed=true; }
   var state=owner.Binding.Snapshot();
   Require(failed&&!state.Applied&&state.RustActiveRevision==null&&state.NativeActiveRevision==null,"failed generation was applied");
   Require(!source.HasListeners()&&!counter.Enabled,"failed generation leaked a listener");
  }
  ClientTelemetry.InjectFailure=null;
  owner.Save(owner.Binding.Snapshot().Revision,BindingTelemetryPreference.On,profile);
  Require(source.HasListeners()&&counter.Enabled,"successful generation has no listener");
  await owner.Operation("Send", _=>Task.FromResult(true));
  await Task.Delay(12000);
  var active=owner.Binding.Snapshot();
  Require(active.Applied&&active.RustActiveRevision==active.NativeActiveRevision,"SDK revisions diverged");
  Require(active.NativeExport.CompletedBatches>0&&active.RustExport.CompletedBatches>0,"missing transport feedback");
  owner.Save(active.Revision,BindingTelemetryPreference.Off,profile);
  Require(!source.HasListeners()&&!counter.Enabled,"off generation left a listener");
  Require(owner.Binding.Snapshot().Applied,"off not acknowledged");
  var before=Volatile.Read(ref writes); gate=true;
  owner.Save(owner.Binding.Snapshot().Revision,BindingTelemetryPreference.On,profile);
  await entered.Task.WaitAsync(TimeSpan.FromSeconds(8));
  await Task.Run(()=>owner.Save(owner.Binding.Snapshot().Revision,BindingTelemetryPreference.Off,profile));
  gate=false; release.TrySetResult(); await Task.Delay(300);
  Require(Volatile.Read(ref writes)==before,"revoked export reached its first HTTP write");
  Require(owner.Binding.Snapshot().Applied&&!source.HasListeners(),"in-flight revocation was not acknowledged");
  owner.Save(owner.Binding.Snapshot().Revision,BindingTelemetryPreference.On,new BindingTelemetryProfile(profile.Endpoint,new Dictionary<string,string>(),0));
  await owner.Operation("Send", async carrier=>{
   Require(carrier!=null&&carrier.Traceparent.EndsWith("-00"),"root ratio zero lost propagation data");
   await Task.Yield();
   await owner.Operation("List", child=>{
    Require(child!=null&&child.Traceparent==carrier!.Traceparent,"nested unsampled operation lost its current context");
    return Task.FromResult(true);
   });
   return true;
  });
  owner.Save(owner.Binding.Snapshot().Revision,BindingTelemetryPreference.On,profile);
  using(var parent=new Activity("synthetic-parent").SetIdFormat(ActivityIdFormat.W3C).SetParentId("00-12345678901234567890123456789012-1234567890123456-00").Start()) {
   parent.TraceStateString="synthetic=acceptance";
   Require(parent.TraceId.ToHexString()=="12345678901234567890123456789012"&&!parent.Recorded,"fixture parent was not an unsampled W3C context: "+parent.Id);
   await owner.Operation("Send", carrier=>{
    Require(carrier!=null&&carrier.Traceparent==parent.Id&&carrier.Tracestate=="synthetic=acceptance","root ratio one lost the unsampled local parent");
    return Task.FromResult(true);
   });
   owner.Save(owner.Binding.Snapshot().Revision,BindingTelemetryPreference.Off,profile);
   await owner.Operation("Send", carrier=>{
    Require(carrier==null,"off propagated an ambient context");
    return Task.FromResult(true);
   });
  }
  owner.Shutdown(); owner.Binding.Dispose();
  Console.WriteLine(JsonSerializer.Serialize(new{passed=true,runId=run,failedAfterTraces=true,failedAfterMetrics=true,noLeakedListeners=true,retryApplied=true,offAcknowledged=true,revokedBeforeFirstWrite=true,unsampledContextPreserved=true}));
 }
}
'@ | Set-Content (Join-Path $root 'Program.cs')
dotnet build (Join-Path $root 'Probe.csproj') -c Release *> (Join-Path $root 'build.log')
if ($LASTEXITCODE -ne 0) { throw 'SDK 验收程序编译失败，见 target/telemetry-sdk-acceptance/build.log' }
$binary = Join-Path $root 'bin\Release\net10.0'
Copy-Item target\release\velune_bindings.dll, target\release\velune_relaykit.dll $binary
& (Join-Path $binary 'Probe.exe') | Set-Content (Join-Path $root 'evidence.json')
if ($LASTEXITCODE -ne 0) { throw 'SDK 验收失败' }
