pub mod adapter;
pub mod routing;
use adapter::{Harness, HarnessAdapter, Submission};
use routing::{Decision, Intent, Resource};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error("invalid operation: {0}")]
    Invalid(&'static str),
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Consumed,
    Queued,
    Unknown,
    Expired,
    AlreadyHandled,
    RouteBlocked,
}
#[derive(Debug, Serialize)]
pub struct Event {
    pub sequence: i64,
    pub message: Option<i64>,
    pub event: String,
    pub simulated: bool,
}
/// Single local host, exclusive SQLite connection. Never expose this trusted API to a model.
/// Future MCP bindings must derive sender from an authenticated process, not tool arguments.
pub struct Core {
    db: Connection,
    epoch: i64,
}
impl Core {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_millis(100))?;
        db.execute_batch(include_str!("schema.sql"))?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("SELECT version FROM meta", [], |r| r.get(0))?;
        if version != 1 {
            return Err(Error::Unsupported("database schema version"));
        }
        tx.execute("UPDATE meta SET epoch=epoch+1", [])?;
        let epoch = tx.query_row("SELECT epoch FROM meta", [], |r| r.get(0))?;
        tx.execute("INSERT INTO events(message,event,simulated) SELECT id,'restart:delivery_unknown',1 FROM messages WHERE state='injecting'",[])?;
        tx.execute("UPDATE tasks SET state='blocked' WHERE id IN (SELECT task FROM messages WHERE state='injecting')",[])?;
        tx.execute(
            "UPDATE messages SET state='unknown' WHERE state='injecting'",
            [],
        )?;
        tx.execute(
            "UPDATE attempts SET state='unknown' WHERE state='injecting'",
            [],
        )?;
        tx.execute(
            "UPDATE runs SET state='reconciling' WHERE state='active'",
            [],
        )?;
        tx.execute(
            "INSERT INTO runs(segment,epoch,state) SELECT id,?,'active' FROM segments",
            [epoch],
        )?;
        tx.commit()?;
        Ok(Self { db, epoch })
    }
    pub fn create_task(&self, goal: &str) -> Result<i64> {
        self.db
            .execute("INSERT INTO tasks(goal,state) VALUES (?,'active')", [goal])?;
        Ok(self.db.last_insert_rowid())
    }
    pub fn session(&mut self, task: i64, harness: Harness, purpose: &str) -> Result<i64> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO sessions(scope,purpose) VALUES (?,?)",
            params![task, purpose],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO segments(session,harness,binding,native_id) VALUES (?,?,?,?)",
            params![
                id,
                harness.name(),
                format!("synthetic-{}", harness.name()),
                format!("mock-session-{id}")
            ],
        )?;
        let segment = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO runs(segment,epoch,state) VALUES (?,?,'active')",
            params![segment, self.epoch],
        )?;
        tx.commit()?;
        Ok(id)
    }
    pub fn discover(&self, caller: i64) -> Result<Vec<(i64, String)>> {
        let mut q=self.db.prepare("SELECT s.id,g.harness FROM sessions s JOIN segments g ON g.session=s.id WHERE s.scope=(SELECT scope FROM sessions WHERE id=?) AND s.id<>? ORDER BY s.id")?;
        Ok(
            q.query_map(params![caller, caller], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?,
        )
    }
    /// This fixed-scope demo permits one level of delegation, never recursive delegation.
    pub fn delegate(
        &mut self,
        sender: i64,
        target: i64,
        key: &str,
        input: &str,
        deadline: i64,
        now: i64,
    ) -> Result<i64> {
        if sender == target || key.is_empty() || deadline <= now {
            return Err(Error::Invalid("target/key/deadline"));
        }
        let tx = self.db.transaction()?;
        let scope:i64=tx.query_row("SELECT a.scope FROM sessions a JOIN sessions b ON a.scope=b.scope WHERE a.id=? AND b.id=?",params![sender,target],|r|r.get(0)).optional()?.ok_or(Error::Invalid("delegation scope"))?;
        let existing: Option<(i64, i64, String, i64)> = tx
            .query_row(
                "SELECT id,target,body,deadline FROM messages WHERE sender=? AND key=?",
                params![sender, key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((id, t, b, d)) = existing {
            if t != target || b != input || d != deadline {
                return Err(Error::Invalid("idempotency conflict"));
            }
            return Ok(id);
        }
        tx.execute("INSERT INTO tasks(parent,goal,state) VALUES (?,'synthetic arithmetic check','planned')",[scope])?;
        let task = tx.last_insert_rowid();
        tx.execute("INSERT INTO messages(task,sender,target,kind,key,body,deadline,state) VALUES (?,?,?,'delegate',?,?,?,'accepted')",params![task,sender,target,key,input,deadline])?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO events(message,event,simulated) VALUES (?,'broker:accepted',1)",
            [id],
        )?;
        tx.commit()?;
        Ok(id)
    }
    pub fn deliver(
        &mut self,
        id: i64,
        adapter: &mut impl HarnessAdapter,
        resources: &[Resource],
        allowed: &[String],
        now: i64,
    ) -> Result<Delivery> {
        let (task, sender, target, kind, body, deadline, state): (
            i64,
            i64,
            i64,
            String,
            String,
            i64,
            String,
        ) = self.db.query_row(
            "SELECT task,sender,target,kind,body,deadline,state FROM messages WHERE id=?",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )?;
        if state == "unknown" || state == "injecting" {
            return Ok(Delivery::Unknown);
        }
        if state != "accepted" {
            return Ok(Delivery::AlreadyHandled);
        }
        if now >= deadline {
            let tx = self.db.transaction()?;
            tx.execute("UPDATE messages SET state='expired' WHERE id=?", [id])?;
            tx.execute("UPDATE tasks SET state='blocked' WHERE id=?", [task])?;
            tx.execute(
                "INSERT INTO events(message,event,simulated) VALUES (?,'deadline:expired',1)",
                [id],
            )?;
            tx.commit()?;
            return Ok(Delivery::Expired);
        }
        let (segment,h,binding,run):(i64,String,String,i64)=self.db.query_row("SELECT g.id,g.harness,g.binding,r.id FROM segments g JOIN runs r ON r.segment=g.id WHERE g.session=? AND r.epoch=? AND r.state='active'",params![target,self.epoch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        let unresolved: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM attempts WHERE segment=? AND state IN ('injecting','unknown'))",[segment],|r|r.get(0))?;
        if unresolved {
            return Ok(Delivery::Unknown);
        }
        let harness = Harness::parse(&h)?;
        if adapter.harness() != harness {
            return Err(Error::Invalid("adapter binding mismatch"));
        }
        let decision = routing::choose(
            &Intent {
                harness,
                allowed_slots: allowed,
                bound_account: Some(&binding),
            },
            resources,
        );
        let Decision::Dispatch(index) = decision else {
            return Ok(Delivery::RouteBlocked);
        };
        let resource = &resources[index];
        let tx = self.db.transaction()?;
        tx.execute("UPDATE messages SET state='injecting' WHERE id=?", [id])?;
        tx.execute("INSERT INTO attempts(message,segment,run,slot,account,model,provider,protocol,state,simulated) VALUES (?,?,?,?,?,?,?,?,'injecting',1)",params![id,segment,run,resource.slot,resource.account,resource.model,resource.provider,format!("{:?}",resource.protocol)])?;
        let attempt = tx.last_insert_rowid();
        tx.execute("INSERT INTO events(message,event,simulated) VALUES (?,'route:dispatch/mock-policy-v1',1)",[id])?;
        tx.commit()?;
        // Deliberate non-transactional boundary, like a native submit. Restart cannot infer no side effect.
        let outcome = adapter.submit(id, &body, kind == "result");
        let tx = self.db.transaction()?;
        let (msg_state, attempt_state, event, delivery) = match &outcome {
            Submission::Busy => ("accepted", "not_sent", "adapter:busy", Delivery::Queued),
            Submission::NotSent => (
                "accepted",
                "not_sent",
                "adapter:confirmed_not_sent",
                Delivery::Queued,
            ),
            Submission::Unknown => (
                "unknown",
                "unknown",
                "adapter:acceptance_unknown",
                Delivery::Unknown,
            ),
            Submission::Consumed(_) => (
                "consumed",
                "completed",
                "adapter:consumed",
                Delivery::Consumed,
            ),
        };
        tx.execute(
            "UPDATE messages SET state=? WHERE id=?",
            params![msg_state, id],
        )?;
        tx.execute(
            "UPDATE attempts SET state=? WHERE id=?",
            params![attempt_state, attempt],
        )?;
        tx.execute(
            "INSERT INTO events(message,event,simulated) VALUES (?,?,1)",
            params![id, event],
        )?;
        if let Submission::Consumed(output) = outcome {
            if kind == "delegate" {
                tx.execute("UPDATE tasks SET state='reported' WHERE id=?", [task])?;
                tx.execute("INSERT INTO events(message,event,simulated) VALUES (?,'delegate:accepted_and_reported',1)",[id])?;
                tx.execute("INSERT INTO messages(task,sender,target,kind,reply_to,key,body,deadline,state) VALUES (?,?,?,'result',?,?,?,?,'accepted')",params![task,target,sender,id,format!("result:{id}"),output,deadline])?;
                let reply = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO events(message,event,simulated) VALUES (?,'result:queued',1)",
                    [reply],
                )?;
            } else {
                tx.execute("INSERT INTO events(message,event,simulated) VALUES (?,'result:received_by_parent',1)",[id])?;
            }
        }
        if msg_state == "unknown" {
            tx.execute("UPDATE tasks SET state='blocked' WHERE id=?", [task])?;
        }
        tx.commit()?;
        Ok(delivery)
    }
    pub fn reply(&self, id: i64) -> Result<Option<i64>> {
        Ok(self
            .db
            .query_row("SELECT id FROM messages WHERE reply_to=?", [id], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn state(&self, id: i64) -> Result<String> {
        Ok(self
            .db
            .query_row("SELECT state FROM messages WHERE id=?", [id], |r| r.get(0))?)
    }
    /// Fixed fixture verifier, separate from harness claimed completion.
    pub fn verify(&self, reply: i64) -> Result<bool> {
        let (task, body, state, kind): (i64, String, String, String) = self.db.query_row(
            "SELECT task,body,state,kind FROM messages WHERE id=?",
            [reply],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        if kind != "result" || state != "consumed" {
            return Err(Error::Invalid("result not consumed"));
        }
        let ok = body == "SIMULATED: arithmetic verified";
        self.db.execute(
            "UPDATE tasks SET state=? WHERE id=?",
            params![if ok { "done" } else { "failed" }, task],
        )?;
        Ok(ok)
    }
    pub fn enqueue_fixture(&mut self, now: i64) -> Result<i64> {
        let pending:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE state IN ('accepted','injecting','unknown'))",[],|r|r.get(0))?;
        if pending {
            return Err(Error::Invalid(
                "finish or cancel pending work first; unknown needs reconciliation",
            ));
        }
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO tasks(goal,state) VALUES ('synthetic arithmetic review','active')",
            [],
        )?;
        let t = tx.last_insert_rowid();
        let mut sessions = Vec::new();
        for h in [Harness::Codex, Harness::ClaudeCode, Harness::Pi] {
            tx.execute(
                "INSERT INTO sessions(scope,purpose) VALUES (?,'synthetic collaborator')",
                [t],
            )?;
            let id = tx.last_insert_rowid();
            sessions.push(id);
            tx.execute(
                "INSERT INTO segments(session,harness,binding,native_id) VALUES (?,?,?,?)",
                params![
                    id,
                    h.name(),
                    format!("synthetic-{}", h.name()),
                    format!("mock-session-{id}")
                ],
            )?;
            let segment = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO runs(segment,epoch,state) VALUES (?,?,'active')",
                params![segment, self.epoch],
            )?;
        }
        for target in &sessions[1..] {
            tx.execute("INSERT INTO tasks(parent,goal,state) VALUES (?,'synthetic arithmetic check','planned')",[t])?;
            let task = tx.last_insert_rowid();
            tx.execute("INSERT INTO messages(task,sender,target,kind,key,body,deadline,state) VALUES (?,?,?,'delegate',?,'2 + 3 = 5',?,'accepted')",params![task,sessions[0],target,format!("fixture:{target}"),now+120])?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO events(message,event,simulated) VALUES (?,'broker:accepted',1)",
                [id],
            )?;
        }
        tx.commit()?;
        Ok(t)
    }
    pub fn tick(&mut self, now: i64) -> Result<()> {
        let next:Option<(i64,String,String)>=self.db.query_row("SELECT m.id,g.harness,m.kind FROM messages m JOIN segments g ON g.session=m.target WHERE m.state='accepted' AND NOT EXISTS(SELECT 1 FROM attempts a WHERE a.segment=g.id AND a.state IN ('injecting','unknown')) ORDER BY m.id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((id, h, kind)) = next {
            let resources: Vec<_> = [Harness::Codex, Harness::ClaudeCode, Harness::Pi]
                .into_iter()
                .map(Resource::fixture)
                .collect();
            let allowed = resources.iter().map(|r| r.slot.clone()).collect::<Vec<_>>();
            let delivery = self.deliver(
                id,
                &mut adapter::SimHarness::new(Harness::parse(&h)?),
                &resources,
                &allowed,
                now,
            )?;
            if kind == "result" && delivery == Delivery::Consumed {
                self.verify(id)?;
            }
        }
        // Recover a crash after result consumption but before fixture verification.
        let replies: Vec<i64> = {
            let mut q=self.db.prepare("SELECT m.id FROM messages m JOIN tasks t ON t.id=m.task WHERE m.kind='result' AND m.state='consumed' AND t.state='reported'")?;
            q.query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        for reply in replies {
            self.verify(reply)?;
        }
        self.db.execute("UPDATE tasks SET state='done' WHERE parent IS NULL AND state='active' AND EXISTS(SELECT 1 FROM tasks c WHERE c.parent=tasks.id) AND NOT EXISTS(SELECT 1 FROM tasks c WHERE c.parent=tasks.id AND c.state<>'done')",[])?;
        Ok(())
    }
    pub fn cancel_pending(&mut self) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("INSERT INTO events(message,event,simulated) SELECT id,'user:cancelled_before_send',1 FROM messages WHERE state='accepted'",[])?;
        tx.execute("UPDATE tasks SET state='cancelled' WHERE id IN (SELECT task FROM messages WHERE state='accepted') AND state<>'blocked'",[])?;
        tx.execute(
            "UPDATE messages SET state='cancelled' WHERE state='accepted'",
            [],
        )?;
        tx.execute("UPDATE tasks SET state='cancelled' WHERE parent IS NULL AND state='active' AND EXISTS(SELECT 1 FROM tasks c WHERE c.parent=tasks.id AND c.state='cancelled')",[])?;
        tx.commit()?;
        Ok(())
    }
    /// Versioned, synthetic-only diagnostics: no input bodies, credentials, or transcripts.
    pub fn diagnostics(&self) -> Result<serde_json::Value> {
        let mut tasks = self
            .db
            .prepare("SELECT id,parent,state FROM tasks ORDER BY id")?;
        let tasks:Vec<_>=tasks.query_map([],|r|Ok(serde_json::json!({"id":r.get::<_,i64>(0)?,"parent":r.get::<_,Option<i64>>(1)?,"state":r.get::<_,String>(2)?})))?.collect::<std::result::Result<_,_>>()?;
        let mut attempts=self.db.prepare("SELECT id,message,segment,run,slot,account,model,protocol,state,provider FROM attempts ORDER BY id")?;
        let attempts:Vec<_>=attempts.query_map([],|r|Ok(serde_json::json!({"id":r.get::<_,i64>(0)?,"message":r.get::<_,i64>(1)?,"segment":r.get::<_,i64>(2)?,"run":r.get::<_,i64>(3)?,"slot":r.get::<_,String>(4)?,"account":r.get::<_,String>(5)?,"model":r.get::<_,String>(6)?,"protocol":r.get::<_,String>(7)?,"state":r.get::<_,String>(8)?,"provider":r.get::<_,String>(9)?})))?.collect::<std::result::Result<_,_>>()?;
        let mut sessions=self.db.prepare("SELECT s.id,s.scope,g.harness FROM sessions s JOIN segments g ON g.session=s.id ORDER BY s.id")?;
        let sessions:Vec<_>=sessions.query_map([],|r|Ok(serde_json::json!({"id":r.get::<_,i64>(0)?,"task":r.get::<_,i64>(1)?,"harness":r.get::<_,String>(2)?})))?.collect::<std::result::Result<_,_>>()?;
        let mut messages = self
            .db
            .prepare("SELECT id,sender,target,kind,reply_to,state FROM messages ORDER BY id")?;
        let messages:Vec<_>=messages.query_map([],|r|Ok(serde_json::json!({"id":r.get::<_,i64>(0)?,"sender":r.get::<_,i64>(1)?,"target":r.get::<_,i64>(2)?,"kind":r.get::<_,String>(3)?,"reply_to":r.get::<_,Option<i64>>(4)?,"state":r.get::<_,String>(5)?})))?.collect::<std::result::Result<_,_>>()?;
        Ok(
            serde_json::json!({"contract_version":1,"core_version":env!("CARGO_PKG_VERSION"),"simulation":true,"native_calls":0,"epoch":self.epoch,"sessions":sessions,"messages":messages,"tasks":tasks,"attempts":attempts,"events":self.events()?}),
        )
    }
    pub fn events(&self) -> Result<Vec<Event>> {
        let mut q = self
            .db
            .prepare("SELECT id,message,event,simulated FROM events ORDER BY id")?;
        Ok(q.query_map([], |r| {
            Ok(Event {
                sequence: r.get(0)?,
                message: r.get(1)?,
                event: r.get(2)?,
                simulated: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
    }
}
