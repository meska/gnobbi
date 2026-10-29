"""Loopback TLS forwarding fixture: commit one reaction, discard its response."""
import http.server
import json
import os
from pathlib import Path
import socket
import sys
import ssl
import urllib.error
import urllib.request

config = json.loads(Path(os.environ["FIXTURE_AUTH_CONFIG"]).read_text())
root = Path(config["ca"]).parent
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({}),
    urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=config["ca"])),
)


class Handler(http.server.BaseHTTPRequestHandler):
    dropped = False

    def log_message(self, format, *args):
        pass

    def forward(self):
        length = int(self.headers.get("Content-Length", "0"))
        if length > 65536:
            self.send_error(413)
            return
        body = self.rfile.read(length) if length else None
        if (self.command == "PUT" and "/redact/" in self.path
                and sys.argv[1:] == ["redaction-before"] and not Handler.dropped):
            Handler.dropped = True
            self.close_connection = True
            self.connection.shutdown(socket.SHUT_RDWR)
            return
        request = urllib.request.Request(
            config["url"].rstrip("/") + self.path,
            data=body,
            method=self.command,
            headers={key: self.headers[key] for key in ("Authorization", "Content-Type") if key in self.headers},
        )
        try:
            response = opener.open(request, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            payload = response.read(1048576)
            status = response.status
        route = "/redact/" if sys.argv[1:] == ["redaction"] else "/send/m.reaction/"
        if self.command == "PUT" and route in self.path and status == 200 and not Handler.dropped:
            Handler.dropped = True
            print(json.dumps({"dropped_event_id": json.loads(payload)["event_id"]}), flush=True)
            self.close_connection = True
            self.connection.shutdown(socket.SHUT_RDWR)
            return
        assert isinstance(status, int)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = forward
    do_POST = forward
    do_PUT = forward


# Threaded: a sync long poll must not block the other requests.
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root / "server.pem", root / "server.key")
server.socket = context.wrap_socket(server.socket, server_side=True)
print(f"https://localhost:{server.server_port}/", flush=True)
server.serve_forever()
