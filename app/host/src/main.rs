use velune_host::{adapter, routing, simulation};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    simulation_main(std::env::args().collect())
}

fn simulation_main(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    use adapter::{Harness, SimHarness};
    use routing::Resource;
    use simulation::Core;

    if args.len() != 3 || !matches!(args[1].as_str(), "demo" | "inspect") {
        return Err("usage: velune-host <demo|inspect> <database-path>".into());
    }
    if args[1] == "demo" && std::path::Path::new(&args[2]).exists() {
        return Err(
            "demo requires a new database; use inspect to recover/read an existing one".into(),
        );
    }
    if args[1] == "inspect" && !std::path::Path::new(&args[2]).exists() {
        return Err("database does not exist".into());
    }
    let mut core = Core::open(&args[2])?;
    if args[1] == "demo" {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/review.json"))?;
        let root = core.create_task(fixture["goal"].as_str().ok_or("fixture goal")?)?;
        let a = core.session(root, Harness::Codex, "coordinator")?;
        let b = core.session(root, Harness::ClaudeCode, "reviewer")?;
        let c = core.session(root, Harness::Pi, "independent checker")?;
        if core.discover(a)?.len() != 2 {
            return Err("discovery failed".into());
        }
        let resources: Vec<_> = [Harness::Codex, Harness::ClaudeCode, Harness::Pi]
            .into_iter()
            .map(Resource::fixture)
            .collect();
        let allowed: Vec<_> = resources.iter().map(|r| r.slot.clone()).collect();
        for (target, kind) in [(b, Harness::ClaudeCode), (c, Harness::Pi)] {
            let id = core.delegate(
                a,
                target,
                kind.name(),
                fixture["input"].as_str().ok_or("fixture input")?,
                100,
                0,
            )?;
            core.deliver(id, &mut SimHarness::new(kind), &resources, &allowed, 1)?;
            let reply = core.reply(id)?.ok_or("missing reply")?;
            core.deliver(
                reply,
                &mut SimHarness::new(Harness::Codex),
                &resources,
                &allowed,
                2,
            )?;
            if !core.verify(reply)? {
                return Err("fixture verification failed".into());
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&core.diagnostics()?)?);
    Ok(())
}
