"""A loopback model provider speaking the OpenAI Responses stream.

Cases register prompts with the reply to stream back and, optionally, a gate
that holds the request until the case releases it. Requests that match no
registered prompt fail loudly, so a case cannot pass on an unexpected call.
"""
from __future__ import annotations

import json
import threading
from collections import Counter
from collections.abc import Callable
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


@dataclass
class Stage:
    prompt: str
    reply: str
    started: threading.Event = field(default_factory=threading.Event)
    released: threading.Event = field(default_factory=threading.Event)
    extra_items: Callable[[], list[dict]] = list


class ResponsesProvider:
    def __init__(self, model: str, hold_seconds: float = 60):
        self.model = model
        self.hold_seconds = hold_seconds
        self.stages: dict[str, Stage] = {}
        self.calls: Counter[str] = Counter()
        self.title_requests = 0
        self.unmatched: list[str] = []
        self._server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self._server.server_port}/v1"

    def stage(self, name: str, prompt: str, reply: str, *, held: bool = False,
              extra_items: Callable[[], list[dict]] = list) -> Stage:
        stage = Stage(prompt, reply, extra_items=extra_items)
        if not held:
            stage.released.set()
        self.stages[name] = stage
        return stage

    def close(self) -> None:
        for stage in self.stages.values():
            stage.released.set()
        self._server.shutdown()
        self._server.server_close()
        self._thread.join()

    def _match(self, request: str) -> str | None:
        return next((name for name, stage in self.stages.items() if stage.prompt in request), None)

    def _response(self, name: str, answer: str, extra: list[dict]) -> bytes:
        item = {"type": "message", "id": f"message-{name}", "status": "completed",
                "role": "assistant", "phase": "final_answer",
                "content": [{"type": "output_text", "text": answer, "annotations": []}]}
        items = extra + [item]
        response = {"id": f"response-{name}", "object": "response", "status": "completed",
                    "model": self.model, "output": items,
                    "usage": {"input_tokens": 16, "output_tokens": 4, "total_tokens": 20}}
        events = [{"type": "response.created", "response": dict(response, status="in_progress", output=[])}]
        for index, entry in enumerate(items):
            events.append({"type": "response.output_item.added", "output_index": index, "item": entry})
            events.append({"type": "response.output_item.done", "output_index": index, "item": entry})
        events.append({"type": "response.completed", "response": response})
        return "".join(f"event: {event['type']}\ndata: {json.dumps(event)}\n\n" for event in events).encode()

    def _handler(self):
        provider = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def write(self, content_type: str, value: bytes) -> None:
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(value)))
                self.end_headers()
                try:
                    self.wfile.write(value)
                except OSError:
                    pass  # The client dropped a held request, e.g. on cancel.

            def do_GET(self):
                if self.path.rstrip("/").endswith("models"):
                    value = {"object": "list", "data": [{"id": provider.model, "object": "model", "owned_by": "synthetic"}]}
                    self.write("application/json", json.dumps(value).encode())
                else:
                    self.send_error(404)

            def do_POST(self):
                if not self.path.split("?", 1)[0].endswith("/responses"):
                    self.send_error(404)
                    return
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                schema = body.get("text", {}).get("format", {}).get("schema", {}).get("properties", {})
                if "Generate task titles only" in str(body.get("instructions", "")) or "title" in schema:
                    provider.title_requests += 1
                    self.write("text/event-stream", provider._response(
                        "title", json.dumps({"title": "Synthetic acceptance conversation"}), []))
                    return
                users = [item for item in body.get("input", []) if item.get("role") == "user"]
                request = json.dumps(users[-1]) if users else ""
                name = provider._match(request)
                if name is None:
                    provider.unmatched.append(request[:200])
                    self.send_error(400, "fixture requires a registered final user prompt")
                    return
                stage = provider.stages[name]
                provider.calls[name] += 1
                stage.started.set()
                if not stage.released.wait(provider.hold_seconds):
                    self.send_error(504, "fixture request held too long")
                    return
                self.write("text/event-stream", provider._response(name, stage.reply, stage.extra_items()))

        return Handler
