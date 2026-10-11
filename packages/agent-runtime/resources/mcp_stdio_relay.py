"""Relay MCP stdio to Velune's JSON-mode loopback HTTP broker using Python 3.

VELUNE_MCP_ENDPOINT and VELUNE_MCP_TOKEN are supplied by the runtime owner.
This is a child transport, with no task state, retries, or SSE support.
"""
import http.client
import json
import os
import socket
import sys
import threading
import urllib.parse


HTTP_TIMEOUT_SECONDS = 60


def visible_ascii(value):
    return isinstance(value, str) and bool(value) and all(
        0x21 <= ord(character) <= 0x7E for character in value
    )


def reject_constant(_value):
    raise ValueError("invalid JSON constant")


def decode_json(value):
    if isinstance(value, bytes):
        value = value.decode("utf-8")
    return json.loads(value, parse_constant=reject_constant)


def valid_id(value):
    return isinstance(value, str) or type(value) is int


def is_response(message):
    if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
        return False
    if "method" in message or not valid_id(message.get("id")):
        return False
    if ("result" in message) == ("error" in message):
        return False
    if "error" in message:
        error = message["error"]
        return (isinstance(error, dict) and type(error.get("code")) is int
                and isinstance(error.get("message"), str))
    return True


def close_socket(connection_socket):
    try:
        connection_socket.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass
    connection_socket.close()


def write_bytes(descriptor, content):
    while content:
        written = os.write(descriptor, content)
        content = content[written:]


class Relay:
    def __init__(self, endpoint, token):
        if not visible_ascii(endpoint) or not visible_ascii(token):
            raise ValueError("invalid configuration")
        address = urllib.parse.urlsplit(endpoint)
        port = address.port
        if (address.scheme != "http" or not port
                or address.netloc != "127.0.0.1:" + str(port)
                or address.fragment):
            raise ValueError("invalid configuration")
        self.port = port
        self.path = urllib.parse.urlunsplit(("", "", address.path or "/", address.query, ""))
        self.token = token
        self.session_id = None
        self.protocol_version = None
        self.state_lock = threading.Lock()
        self.output_lock = threading.Lock()
        self.stopped = threading.Event()
        self.sockets = set()

    def stop(self):
        self.stopped.set()
        with self.state_lock:
            sockets = list(self.sockets)
        for connection_socket in sockets:
            close_socket(connection_socket)

    def emit(self, message):
        payload = json.dumps(message, ensure_ascii=True, allow_nan=False, separators=(",", ":"))
        with self.output_lock:
            if self.stopped.is_set():
                return
            try:
                write_bytes(1, payload.encode("utf-8") + b"\n")
            except OSError:
                # A disconnected stdio parent cannot consume any further result.
                self.stop()
                os._exit(0)

    def fail(self, request_id, expects_response, message, code=-32000):
        if self.stopped.is_set():
            return
        if expects_response:
            self.emit({"jsonrpc": "2.0", "id": request_id,
                       "error": {"code": code, "message": message}})
        else:
            try:
                write_bytes(2, ("Velune MCP relay: " + message + "\n").encode("ascii"))
            except OSError:
                pass

    def accept(self, line):
        try:
            message = decode_json(line)
        except (ValueError, UnicodeError, RecursionError):
            self.fail(None, True, "Invalid JSON input", -32700)
            return
        request_id = message.get("id") if isinstance(message, dict) else None
        valid_request = (isinstance(message, dict) and message.get("jsonrpc") == "2.0"
                         and isinstance(message.get("method"), str) and bool(message["method"])
                         and "result" not in message and "error" not in message
                         and ("id" not in message or valid_id(request_id)))
        if not valid_request and not is_response(message):
            self.fail(request_id if valid_id(request_id) else None, True,
                      "Invalid JSON-RPC input", -32600)
            return
        expects_response = valid_request and "id" in message
        # Each POST waits independently, so a pending wait never blocks cancel/status.
        threading.Thread(target=self.forward, args=(message, expects_response), daemon=True).start()

    def forward(self, message, expects_response):
        request_id = message.get("id")
        connection = http.client.HTTPConnection("127.0.0.1", self.port,
                                                timeout=HTTP_TIMEOUT_SECONDS)
        connection_socket = None
        expired = threading.Event()

        def expire():
            expired.set()
            if connection_socket is not None:
                close_socket(connection_socket)

        deadline = threading.Timer(HTTP_TIMEOUT_SECONDS, expire)
        deadline.daemon = True
        deadline.start()
        try:
            connection.connect()
            connection_socket = connection.sock
            with self.state_lock:
                if self.stopped.is_set():
                    return
                if expired.is_set():
                    raise TimeoutError
                self.sockets.add(connection_socket)
                session_id = self.session_id
                protocol_version = self.protocol_version
            headers = {"Authorization": "Bearer " + self.token,
                       "Content-Type": "application/json",
                       "Accept": "application/json, text/event-stream"}
            if session_id is not None and message.get("method") != "initialize":
                headers["Mcp-Session-Id"] = session_id
            if protocol_version is not None:
                headers["MCP-Protocol-Version"] = protocol_version
            payload = json.dumps(message, ensure_ascii=True, allow_nan=False).encode("utf-8")
            connection.request("POST", self.path, body=payload, headers=headers)
            response = connection.getresponse()
            if response.status < 200 or response.status >= 300:
                self.fail(request_id, expects_response,
                          "MCP HTTP request failed (" + str(response.status) + ")")
                return
            if not expects_response:
                if response.status not in (202, 204):
                    self.fail(request_id, False, "MCP notification was not acknowledged")
                return
            content_type = response.getheader("Content-Type", "").split(";", 1)[0].strip().lower()
            if content_type == "text/event-stream":
                self.fail(request_id, True, "MCP relay requires a JSON-mode broker; SSE is unsupported")
                return
            if content_type != "application/json":
                self.fail(request_id, True, "MCP HTTP response is not application/json")
                return
            result = decode_json(response.read())
            if not is_response(result) or result["id"] != request_id:
                self.fail(request_id, True, "Invalid MCP JSON-RPC response")
                return
            if message.get("method") == "initialize" and "result" in result:
                initialized = result["result"]
                version = initialized.get("protocolVersion") if isinstance(initialized, dict) else None
                session = response.getheader("Mcp-Session-Id")
                if not visible_ascii(version) or (session is not None and not visible_ascii(session)):
                    self.fail(request_id, True, "Invalid MCP initialization response")
                    return
                with self.state_lock:
                    self.session_id = session
                    self.protocol_version = version
            self.emit(result)
        except (OSError, http.client.HTTPException, ValueError, UnicodeError, RecursionError) as error:
            self.fail(request_id, expects_response,
                      "MCP HTTP request timed out" if expired.is_set() or isinstance(error, TimeoutError)
                      else "MCP HTTP transport or protocol failure")
        finally:
            deadline.cancel()
            with self.state_lock:
                self.sockets.discard(connection_socket)
            connection.close()
            if connection_socket is not None:
                close_socket(connection_socket)


def main():
    try:
        relay = Relay(os.environ.get("VELUNE_MCP_ENDPOINT", ""),
                      os.environ.get("VELUNE_MCP_TOKEN", ""))
    except ValueError:
        write_bytes(2, b"Velune MCP relay requires a loopback HTTP endpoint and token\n")
        return 2
    try:
        for line in sys.stdin.buffer:
            relay.accept(line)
    finally:
        # Workers are daemon threads: EOF also ends a blocked connect/read immediately.
        relay.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
