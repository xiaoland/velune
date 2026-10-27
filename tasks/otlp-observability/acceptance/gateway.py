"""Real Runner HTTP failover, concurrent parent capture and SDK replacement."""
import http.client
import json
import shutil
from pathlib import Path
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


PROBE = r'''
use std::{collections::BTreeMap,io::{BufRead,Write},sync::Arc};
use velune_telemetry::{ClientTelemetry,ResourceIdentity,Profile,Preference,Outcome};
struct Resolver;
impl velune_gateway::CredentialResolver for Resolver {
 fn resolve(&self,_:String,_:velune_gateway::CredentialTarget)
 ->velune_ai::OperationFuture<Result<velune_gateway::ResolvedCredential,velune_gateway::CredentialResolutionError>> {
  Box::pin(async {Ok(velune_gateway::ResolvedCredential{material:velune_gateway::CredentialMaterial::Http{
   authorization:velune_gateway::HttpAuthorization::Bearer("CANARY-provider-secret".into()),account_id:None},
   recovery:velune_gateway::CredentialRecovery::None,request_capabilities:velune_gateway::RequestCapabilities::default()})})
 }
}
fn activate(owner:&ClientTelemetry,endpoint:&str,header:&str)->Result<(),Box<dyn std::error::Error>> {
 let profile=Profile{endpoint:endpoint.into(),headers:BTreeMap::from([("x-synthetic-authorization".into(),header.into())]),sample_ratio:1.0};
 let p=owner.prepare(owner.snapshot()?.revision,Preference::On,Some(profile))?;
 owner.commit(&p.id)?;owner.confirm_native(p.revision,true)?;Ok(())
}
fn main()->Result<(),Box<dyn std::error::Error>> {
 let args:Vec<_>=std::env::args().collect();
 let owner=ClientTelemetry::open(Path::new(&args[1]),ResourceIdentity::new("test","synthetic")?,true,None)?;
 activate(&owner,&args[3],"first")?;
 let config=serde_json::from_value(serde_json::json!({"execution":{"kind":"local"},"os_home":args[1],"runtime_type_id":"pi-1.0",
   "binary":args[5],"node_binary":args[5],"rpc_entry":args[6]}))?;
 let mut startup=owner.operation("harness.start",None,false);
 let mut pi=startup.run(||velune_agent_runtime::Client::spawn(config))?;
 pi.request(serde_json::json!({"type":"synthetic_ping"}))?;
 startup.complete(Outcome::Success);drop(startup);assert!(owner.flush());
 let runner=velune_gateway::Runner::start(serde_json::from_slice(&std::fs::read(&args[2])?)?,Arc::new(Resolver),BTreeMap::new())?;
 let mut turns=BTreeMap::new();
 println!("{}",serde_json::json!({"endpoint":runner.endpoint(),"token":runner.token(),"instanceId":owner.snapshot()?.instance_id}));
 std::io::stdout().flush()?;
 for line in std::io::stdin().lock().lines() {
  let command:serde_json::Value=serde_json::from_str(&line?)?;
  let result=match command["action"].as_str().unwrap_or_default() {
   "bind"=>{
    let turn=owner.operation("execution.turn",None,false);
    let carrier=turn.carrier();runner.set_telemetry(Some(owner.clone()),carrier.clone(),turn.revision());
    turns.insert(command["id"].as_str().unwrap().to_owned(),turn);
    serde_json::json!({"parent":carrier.map(|c|c.traceparent)})
   },
   "clear"=>{runner.set_telemetry(Some(owner.clone()),None,None);serde_json::json!({"ok":true})},
   "switch"=>{activate(&owner,&args[4],"second")?;pi.request(serde_json::json!({"type":"synthetic_ping"}))?;serde_json::json!({"ok":true})},
   "flush"=>serde_json::json!({"ok":owner.flush()}),
   "stop"=>break,
   _=>return Err("unknown fixture action".into()),
  };
  println!("{result}");std::io::stdout().flush()?;
 }
 drop(runner);
 pi.shutdown()?;
 for turn in turns.values_mut(){turn.complete(Outcome::Success);}
 drop(turns);owner.flush();owner.shutdown()?;Ok(())
}
use std::path::Path;
'''


def run_gateway(repository, directory, ports, library):
    arrivals = {name: threading.Event() for name in ("one", "two")}
    release = threading.Event()
    captures = []

    class Upstream(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            case = body["fixture_case"]
            captures.append((case, self.path))
            if case in arrivals:
                arrivals[case].set()
                if not release.wait(15):
                    self.send_error(504)
                    return
            status = 503 if self.path.startswith("/first/") else 200
            payload = b'{"error":{"message":"CANARY-upstream-error"}}' if status == 503 else b'{"choices":[]}'
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    process = None
    try:
        providers = [{"id": name, "name": name, "protocol": "chatCompletionsV1", "executionBackend": "http",
                      "requestProfile": "native", "endpoint": f"http://127.0.0.1:{server.server_port}/{name}/v1",
                      "credentialRef": "synthetic", "quotaLimits": [], "models": [{"recordKey": name, "providerModelId": "synthetic", "nickname": name}]}
                     for name in ("first", "second")]
        config = directory / "gateway.json"
        config.write_text(json.dumps({"id": "synthetic", "name": "synthetic", "providers": providers,
                                     "routes": [{"recordKey": "retry", "name": "retry", "members": ["first", "second"], "strategy": "order", "sticky": False, "weights": {}}]}))
        source = directory / "gateway.rs"
        source.write_text(PROBE)
        node = shutil.which("node")
        assert node, "the synthetic Pi adapter fixture needs the installed Node runtime"
        peer = directory / "synthetic-pi.mjs"
        peer.write_text("import {createInterface} from 'node:readline';\nfor await (const line of createInterface({input:process.stdin})) {const r=JSON.parse(line); console.log(JSON.stringify({type:'response',id:r.id,command:r.type,success:true,data:{}}));}\n")
        build = subprocess.run(["cargo", "build", "--locked", "-p", "velune-bindings", "--lib", "--message-format=json"], cwd=repository, capture_output=True, text=True, check=True)
        artifacts = {}
        for line in build.stdout.splitlines():
            item = json.loads(line)
            if item.get("reason") == "compiler-artifact":
                for filename in item["filenames"]:
                    if filename.endswith(".rlib"):
                        artifacts[item["target"]["name"]] = filename
        command = ["rustc", "--edition=2024", str(source), "-L", f"dependency={library.parent / 'deps'}"]
        for name in ("velune_gateway", "velune_ai", "velune_telemetry", "velune_agent_runtime", "serde_json"):
            command.extend(("--extern", f"{name}={artifacts[name]}"))
        command.extend(("-o", str(directory / "gateway")))
        subprocess.run(command, cwd=repository, check=True)
        with (directory / "gateway-error.log").open("w") as errors:
            process = subprocess.Popen([str(directory / "gateway"), str(directory / "gateway-home"), str(config), *[f"http://127.0.0.1:{port}" for port in ports], node, str(peer)],
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True)
            info = json.loads(process.stdout.readline())

            def control(action, **values):
                process.stdin.write(json.dumps({"action": action, **values}) + "\n"); process.stdin.flush()
                return json.loads(process.stdout.readline())

            def request(case, model="second"):
                connection = http.client.HTTPConnection("127.0.0.1", int(info["endpoint"].split(":")[2].split("/")[0]), timeout=20)
                connection.request("POST", "/v1/chat/completions", json.dumps({"model": "velune/model/" + model, "fixture_case": case,
                                   "messages": [{"role": "user", "content": "CANARY-private-prompt"}]}),
                                   {"Content-Type": "application/json", "Authorization": "Bearer " + info["token"]})
                response = connection.getresponse()
                assert response.status == 200, (case, response.status)
                assert json.loads(response.read()) == {"choices": []}
                connection.close()

            parents = {"retry": control("bind", id="retry")["parent"]}
            request("retry", "retry")
            assert [path for case, path in captures if case == "retry"] == ["/first/v1/chat/completions", "/second/v1/chat/completions"]
            assert control("flush")["ok"]
            errors_in_threads = []

            def held(case):
                try:
                    request(case)
                except Exception as error:
                    errors_in_threads.append(str(error))

            threads = []
            for name in ("one", "two"):
                parents[name] = control("bind", id=name)["parent"]
                thread = threading.Thread(target=held, args=(name,)); thread.start(); threads.append(thread)
                assert arrivals[name].wait(5), "request did not enter its upstream"
            assert parents["one"] != parents["two"]
            assert control("switch")["ok"]
            release.set()
            for thread in threads:
                thread.join(10)
                assert not thread.is_alive()
            assert not errors_in_threads, errors_in_threads
            assert control("clear")["ok"]
            request("missing")
            assert control("flush")["ok"]
            process.stdin.write('{"action":"stop"}\n'); process.stdin.flush()
            assert process.wait(timeout=10) == 0, "gateway fixture failed"
            failed_attempt = None
            for path in (directory / "gateway-home/logs").glob("*.jsonl"):
                text = path.read_text()
                assert "CANARY" not in text, "private HTTP input reached the local diagnostic writer"
                for line in text.splitlines():
                    record = json.loads(line)
                    if record.get("fields", {}).get("http_status") == 503:
                        failed_attempt = record.get("span", {}).get("operation_id")
            assert failed_attempt, "local upstream status has no operation identity"
            return {"instanceId": info["instanceId"], "parents": parents, "failoverAttempts": 2, "sameRunnerReconfigured": True,
                    "readerAliveAcrossSwitch": True, "failedAttemptOperationId": failed_attempt}
    finally:
        release.set()
        if process is not None and process.poll() is None:
            process.kill(); process.wait()
        server.shutdown(); server.server_close()
