"""An existing Codex conversation across send, cold reopen, an external
writer, queue, stop and restart. Real Codex CLI, loopback model, private HOME.

Migrated from scripts/check-conversation-parity.py; run it against two builds
and compare the manifests to check parity between them.
"""
from __future__ import annotations

import fcntl
import json
from collections import Counter

from velune_acceptance import Context
from velune_acceptance.app import Driver, texts
from velune_acceptance.responses import ResponsesProvider

REQUIRES = ("codex",)
RUNTIME = "acceptance-codex"
MODEL = "gpt-6.1-sol"
STAGES = ("seed", "resume", "queue", "stop", "recover")
PROMPT = {stage: f"ACCEPTANCE-INPUT-{stage}" for stage in STAGES}
REPLY = {stage: f"ACCEPTANCE-OUTPUT-{stage}" for stage in STAGES}
HELD = {"resume", "stop"}


def codex_config(provider: ResponsesProvider) -> str:
    return f'''model = "{MODEL}"
model_provider = "synthetic"
approval_policy = "never"
sandbox_mode = "read-only"
model_post_turn_compact_threshold_percent = 0
[model_providers.synthetic]
name = "Synthetic local provider"
base_url = "{provider.base_url}"
wire_api = "responses"
experimental_bearer_token = "synthetic-fake-token"
requires_openai_auth = false
[features]
responses_websockets = false
responses_websockets_v2 = false
hooks = false
goals = false
plugins = false
apps = false
memories = false
shell_snapshot = false
'''


def run(ctx: Context) -> None:
    codex_binary = ctx.tool("codex")
    b = ctx.bindings
    api, conversation = b.application_api, b.conversation
    provider = ResponsesProvider(MODEL)
    for stage in STAGES:
        provider.stage(stage, PROMPT[stage], REPLY[stage], held=stage in HELD)
    codex = ctx.isolated.dir("codex")
    (codex / "config.toml").write_text(codex_config(provider))

    options = b.bindings.BindingOptions(home_directory=str(ctx.isolated.state),
                                        resources_directory=str(ctx.isolated.resources))
    runtime = api.RuntimeInstance(id=RUNTIME, name="Synthetic Codex", enabled=True,
        type_id=conversation.RuntimeKind.CODEX166, gateway_id="default",
        settings={"binary": str(codex_binary), "agentDir": str(codex), "nodeBinary": "/usr/bin/false"})
    execution = api.RuntimeExecution(transport=api.RuntimeTransport.LOCAL(),
        home=api.RuntimeHome.EXISTING(path=str(ctx.isolated.home)), remote_resources_directory=None)
    model = conversation.ModelSelection.NATIVE(provider_id="synthetic", model_id=MODEL)
    driver = Driver(b, options, RUNTIME, ctx.timeout)
    calls = lambda: f"provider calls={dict(provider.calls)} unmatched={provider.unmatched}"

    def compose(stage):
        return api.ComposedTurn(text=PROMPT[stage], effort=None, permission_mode=None, attachments=[])

    def submit(stage):
        return driver.app.send_turn(RUNTIME, driver.key, RUNTIME, None, compose(stage),
                                    submission_id=f"acceptance-{stage}")

    try:
        driver.app.upsert_runtime_with_execution(runtime, execution)
        created = driver.app.create_conversation(RUNTIME, str(ctx.isolated.project), model).snapshot
        driver.key = created.conversation.id
        submit("seed")
        driver.wait("new conversation must send and settle",
                    lambda s: s.run_state.name == "IDLE" and REPLY["seed"] in texts(s), calls)
        ctx.passed("new_send_completed")

        rollouts = list((codex / "sessions").rglob("rollout-*.jsonl"))
        assert len(rollouts) == 1, rollouts
        driver.reopen()
        reopened = driver.wait("cold reopen must keep the same conversation and history",
                               lambda s: s.actions.can_send and REPLY["seed"] in texts(s), calls)
        assert reopened.conversation.id == driver.key and reopened.context_runtime_id == RUNTIME
        assert texts(reopened).count(PROMPT["seed"]) == texts(reopened).count(REPLY["seed"]) == 1
        ctx.passed("cold_reopen_history_and_send_access")

        # Hold the writer lock the Codex adapter observes. This models an
        # external controller without running an external turn.
        with rollouts[0].open() as rollout:
            native_id = json.loads(rollout.readline())["payload"]["id"]
        lock = codex / "thread-writer-locks" / f"{native_id}.lock"
        lock.parent.mkdir(exist_ok=True)
        with lock.open("a+") as writer:
            fcntl.flock(writer, fcntl.LOCK_EX | fcntl.LOCK_NB)
            driver.app.open_conversation(RUNTIME, driver.key)
            held = driver.wait("refresh must publish the external writer",
                               lambda s: s.actions.input_access.name == "EXTERNAL_CONTROLLER")
            assert not held.actions.can_send and not held.actions.can_cancel
            assert REPLY["seed"] in texts(held)
        driver.app.open_conversation(RUNTIME, driver.key)
        driver.wait("release and refresh must restore send access", lambda s: s.actions.can_send)
        ctx.passed("external_writer_refresh_recovers")

        submit("resume")
        assert provider.stages["resume"].started.wait(ctx.timeout), "resume did not reach the model"
        active = driver.wait("active turn must offer cancel",
                             lambda s: s.run_state.name == "RUNNING" and s.actions.can_cancel)
        assert PROMPT["seed"] in texts(active) and REPLY["seed"] in texts(active), "resume dropped history"
        ctx.passed("resumed_turn_preserves_history_and_is_stoppable")

        operation = api.SendQueueOperation.ADD(source_runtime_id=RUNTIME, target_runtime_id=RUNTIME,
                                               selection=None, turn=compose("queue"))
        result = driver.app.send_queue(RUNTIME, driver.key, operation, submission_id="acceptance-queue")
        assert result.submission_receipt.is_QUEUED(), result.submission_receipt
        queued = driver.wait("queued input must have a visible queue entry", lambda s: bool(s.follow_up.queue))
        assert [item.text for item in queued.follow_up.queue] == [PROMPT["queue"]]
        assert PROMPT["queue"] not in texts(queued) and not provider.stages["queue"].started.is_set()
        ctx.passed("accepted_queue_has_no_early_transcript_entry")

        provider.stages["resume"].released.set()
        drained = driver.wait("successful turn must drain its queued input",
                              lambda s: s.run_state.name == "IDLE" and REPLY["queue"] in texts(s), calls)
        expected = [value for stage in ("seed", "resume", "queue") for value in (PROMPT[stage], REPLY[stage])]
        assert [text for text in texts(drained) if text in expected] == expected and not drained.follow_up.queue
        ctx.passed("queue_drains_once_in_native_order")

        submit("stop")
        assert provider.stages["stop"].started.wait(ctx.timeout), "stop turn did not reach the model"
        driver.wait("pending answer must be cancellable", lambda s: s.actions.can_cancel)
        driver.app.cancel(RUNTIME, driver.key)
        cancelled = driver.wait("interrupt must release active state",
                                lambda s: s.run_state.name == "IDLE" and not s.actions.can_cancel)
        assert REPLY["stop"] not in texts(cancelled)
        ctx.passed("stop_releases_active_state_without_an_answer")

        driver.reopen()
        after_stop = driver.wait("restart after stop must restore history and access",
                                 lambda s: s.actions.can_send and REPLY["queue"] in texts(s))
        assert [text for text in texts(after_stop) if text in expected] == expected
        assert REPLY["stop"] not in texts(after_stop)
        submit("recover")
        recovered = driver.wait("reopened conversation must accept the next turn",
                                lambda s: s.run_state.name == "IDLE" and REPLY["recover"] in texts(s), calls)
        assert recovered.conversation.id == driver.key and recovered.context_runtime_id == RUNTIME
        assert texts(recovered).count(PROMPT["recover"]) == texts(recovered).count(REPLY["recover"]) == 1
        assert provider.calls == Counter({stage: 1 for stage in STAGES}), provider.calls
        ctx.passed("reopen_after_stop_and_send_once")
        driver.close_when_idle()
    except BaseException:
        for stage in provider.stages.values():
            stage.released.set()
        driver.abandon()
        raise
    finally:
        ctx.facts["provider_calls"] = dict(provider.calls)
        ctx.facts["title_requests"] = provider.title_requests
        provider.close()
