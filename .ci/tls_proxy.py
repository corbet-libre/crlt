"""CI-only TLS termination and transport faults in front of the real sqld."""

import http.client
import http.server
import json
from pathlib import Path
import socket
import ssl
import sys
import threading
import time


class Proxy(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass  # Do not log SQL, authentication headers or requests.

    def do_POST(self):
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        body = self.rfile.read(int(self.headers["Content-Length"]))
        statements = [request.get("stmt", {}).get("sql", "")
                      for request in json.loads(body).get("requests", [])]
        if self.headers.get("User-Agent") != "crlt":
            self.send_error(400)
            return
        prefix = self.path.split("/", 2)[1]
        if prefix == "redirect":
            self.send_response(307)
            self.send_header("Location", "https://localhost:18443")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if prefix == "delay":
            time.sleep(2)
        path = self.path[self.path.index("/v3/"):]
        upstream = http.client.HTTPConnection("127.0.0.1", 18080, timeout=20)
        try:
            upstream.request("POST", path, body, {
                "Authorization": self.headers.get("Authorization", ""),
                "Content-Type": "application/json",
            })
            response = upstream.getresponse()
            payload = response.read()
            # Faults happen after the real server executes the request. These
            # exercise uncertain delivery, cancellation and non-idempotent writes.
            if b"crlt-delay-query" in body:
                time.sleep(2)
            if b"crlt-unknown-write" in body and any(sql.startswith("UPDATE") for sql in statements):
                self.send_response(503)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if prefix in ("downgrade", "userinfo") and response.status == 200:
                document = json.loads(payload)
                document["base_url"] = (
                    "http://127.0.0.1:18080" if prefix == "downgrade"
                    else "https://test:secret@localhost:18443"
                )
                payload = json.dumps(document).encode()
            if prefix in ("missing-baton", "missing-status") and response.status == 200:
                document = json.loads(payload)
                if prefix == "missing-baton":
                    document["baton"] = None
                else:
                    document["results"] = document["results"][:1]
                payload = json.dumps(document).encode()
            if response.status == 200:
                document = json.loads(payload)
                # Preserve the fault endpoint across real server baton changes.
                if prefix in ("foreign-keys-off", "missing-foreign-keys", "begin-state",
                              "commit-state", "rollback-state"):
                    document["base_url"] = "https://localhost:18443/" + prefix
                results = document.get("results", [])
                if len(results) == 2 and results[0].get("type") == "ok":
                    result = results[0]["response"].get("result")
                    if result is not None:
                        if prefix == "missing-health":
                            result["rows"] = []
                        elif prefix == "wrong-health":
                            result["rows"][0][0] = {"type": "integer", "value": "2"}
                        elif prefix == "wrong-width":
                            result["rows"][0] = []
                        elif prefix == "invalid-value":
                            result["rows"][0][0] = {"type": "none"}
                        elif "PRAGMA foreign_keys" in statements:
                            if prefix == "foreign-keys-off":
                                result["rows"][0][0] = {"type": "integer", "value": "0"}
                            elif prefix == "missing-foreign-keys":
                                result["rows"] = []
                    if prefix == "wrong-status":
                        results[1]["response"] = {"type": "close"}
                    elif prefix == "wrong-response":
                        results[0]["response"] = {"type": "close"}
                    elif prefix == "begin-state" and "BEGIN IMMEDIATE" in statements:
                        results[1]["response"]["is_autocommit"] = True
                    elif prefix in ("commit-state", "rollback-state"):
                        action = "COMMIT" if prefix == "commit-state" else "ROLLBACK"
                        if action in statements:
                            results[1]["response"]["is_autocommit"] = False
                if prefix in ("query-url", "fragment-url", "password-url"):
                    document["base_url"] = {
                        "query-url": "https://localhost:18443?forbidden=1",
                        "fragment-url": "https://localhost:18443#forbidden",
                        "password-url": "https://:secret@localhost:18443",
                    }[prefix]
                if prefix == "missing-base":
                    document.pop("base_url", None)
                payload = json.dumps(document).encode()
            self.send_response(response.status)
            self.send_header("Content-Type", response.getheader("Content-Type", "application/json"))
            if prefix == "oversize-length":
                self.send_header("Content-Length", str(16 * 1024 * 1024 + 1))
                self.end_headers()
                return
            if prefix == "oversize-stream":
                # No declared length: enforce the streaming limit independently.
                self.end_headers()
                self.wfile.write(b" " * (16 * 1024 * 1024 + 1))
                return
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            if prefix == "truncated-body":
                self.wfile.write(payload[:len(payload) // 2])
                self.close_connection = True
                return
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected when the deadline test cancels its request.
        finally:
            upstream.close()


fixture = Path(sys.argv[1])
for port, certificate, key in [
    (18443, "server.crt", "server.key"),
    (18444, "untrusted.crt", "untrusted.key"),
]:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Proxy)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(fixture / certificate, fixture / key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
threading.Event().wait()
