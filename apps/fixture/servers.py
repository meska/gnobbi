#!/usr/bin/env python3
"""Disposable Synapse + Tuwunel behind one TLS proxy, for the iOS/Android sample apps.

  servers.py up [--host IP]   start both, register gnobbi-test-* accounts, create rooms, print JSON
  servers.py down             remove containers and the state dir

Needs docker. Everything (CA, passwords, data) lives in the git-ignored state dir (default
apps/fixture/.run); passwords are random per run. The apps trust the generated CA only.
"""
import argparse, json, secrets, shutil, socket, ssl, subprocess, sys, time, urllib.error, urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUN = HERE / ".run"
NET = "gnobbi-fixture"
SYNAPSE, TUWUNEL = "matrixdotorg/synapse:latest", "ghcr.io/matrix-construct/tuwunel:latest"
PORTS = {"synapse": 18448, "tuwunel": 18449}
NAME = "gnobbi.test"
TOKEN = secrets.token_hex(8)


def sh(*a, check=True, **k):
    return subprocess.run(a, check=check, text=True, capture_output=True, **k)


def lan_ip():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.connect(("192.0.2.1", 9))  # no packet is sent
    return s.getsockname()[0]


def tls(host):
    ca_key, ca, key, csr, crt, ext = (RUN / n for n in ("ca.key", "ca.pem", "srv.key", "srv.csr", "srv.pem", "ext"))
    sh("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(ca_key), "-out", str(ca),
       "-days", "3", "-subj", "/CN=gnobbi-fixture-ca", "-addext", "basicConstraints=critical,CA:TRUE",
       "-addext", "keyUsage=critical,keyCertSign")
    sh("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key), "-out", str(csr), "-subj", "/CN=gnobbi-fixture")
    ext.write_text(f"subjectAltName=DNS:localhost,IP:127.0.0.1,IP:10.0.2.2,IP:{host}\nextendedKeyUsage=serverAuth\n")
    sh("openssl", "x509", "-req", "-in", str(csr), "-CA", str(ca), "-CAkey", str(ca_key), "-CAcreateserial",
       "-out", str(crt), "-days", "3", "-extfile", str(ext))


def call(base, ca, method, path, body=None, token=None):
    ctx = ssl.create_default_context(cafile=str(ca))
    req = urllib.request.Request(base + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {token}"} if token else {})})
    try:
        with urllib.request.urlopen(req, context=ctx, timeout=20) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        return {"_status": e.code, **json.loads(e.read() or b"{}")}


def register(base, ca, user, password, server):
    body = {"username": user, "password": password, "inhibit_login": False}
    first = call(base, ca, "POST", "/_matrix/client/v3/register", body)
    if "access_token" in first:
        return first
    auth = {"type": "m.login.registration_token", "token": TOKEN, "session": first.get("session")} \
        if server == "tuwunel" else {"type": "m.login.dummy", "session": first.get("session")}
    done = call(base, ca, "POST", "/_matrix/client/v3/register", {**body, "auth": auth})
    if "access_token" not in done:
        done = call(base, ca, "POST", "/_matrix/client/v3/register", {**body, "auth": {"type": "m.login.dummy", "session": first.get("session")}})
    assert "access_token" in done, f"register {server}: {done}"
    return done


def up(host):
    shutil.rmtree(RUN, ignore_errors=True)
    for d in ("synapse", "tuwunel"):
        (RUN / d).mkdir(parents=True)
    tls(host)
    sh("docker", "network", "create", NET, check=False)
    # Synapse: generate the config, then open registration (test server only).
    sh("docker", "run", "--rm", "-e", f"SYNAPSE_SERVER_NAME={NAME}", "-e", "SYNAPSE_REPORT_STATS=no",
       "-v", f"{RUN/'synapse'}:/data", SYNAPSE, "generate")
    with open(RUN / "synapse/homeserver.yaml", "a") as f:
        f.write("\nenable_registration: true\nenable_registration_without_verification: true\n"
                "rc_message: {per_second: 100, burst_count: 100}\nrc_registration: {per_second: 100, burst_count: 100}\n"
                "rc_joins: {local: {per_second: 100, burst_count: 100}}\nrc_login: {address: {per_second: 100, burst_count: 100},"
                " account: {per_second: 100, burst_count: 100}, failed_attempts: {per_second: 100, burst_count: 100}}\n")
    sh("docker", "run", "-d", "--name", f"{NET}-synapse", "--network", NET, "-v", f"{RUN/'synapse'}:/data", SYNAPSE)
    sh("docker", "run", "-d", "--name", f"{NET}-tuwunel", "--network", NET,
       "-e", f"TUWUNEL_SERVER_NAME={NAME}", "-e", "TUWUNEL_ADDRESS=0.0.0.0", "-e", "TUWUNEL_PORT=6167",
       "-e", "TUWUNEL_DATABASE_PATH=/var/lib/tuwunel", "-e", "TUWUNEL_ALLOW_REGISTRATION=true",
       "-e", f"TUWUNEL_REGISTRATION_TOKEN={TOKEN}", "-e", "TUWUNEL_ALLOW_FEDERATION=false", TUWUNEL)
    (RUN / "Caddyfile").write_text("".join(
        f"https://:{p} {{\n  tls /etc/c/srv.pem /etc/c/srv.key\n  reverse_proxy {NET}-{n}:{b}\n}}\n"
        for n, p, b in (("synapse", PORTS["synapse"], 8008), ("tuwunel", PORTS["tuwunel"], 6167))))
    sh("docker", "run", "-d", "--name", f"{NET}-proxy", "--network", NET, "-v", f"{RUN}:/etc/c:ro",
       "-p", f"{PORTS['synapse']}:{PORTS['synapse']}", "-p", f"{PORTS['tuwunel']}:{PORTS['tuwunel']}",
       "caddy:2", "caddy", "run", "--config", "/etc/c/Caddyfile")
    ca = RUN / "ca.pem"
    out = {"ca": str(ca), "servers": {}}
    for server, port in PORTS.items():
        base = f"https://127.0.0.1:{port}"
        for _ in range(60):
            try:
                if "versions" in call(base, ca, "GET", "/_matrix/client/versions"):
                    break
            except Exception:
                pass
            time.sleep(2)
        else:
            sys.exit(f"{server} did not come up")
        pw = {u: secrets.token_urlsafe(18) for u in ("a", "b")}
        acc = {u: register(base, ca, f"gnobbi-test-{u}", pw[u], server) for u in pw}
        a = acc["a"]["access_token"]
        rooms = []
        for i in (1, 2, 3):
            r = call(base, ca, "POST", "/_matrix/client/v3/createRoom", {"name": f"Stanza prova {i}", "preset": "private_chat",
                     "invite": [f"@gnobbi-test-b:{NAME}"] if i == 1 else []}, a)
            rooms.append(r["room_id"])
            call(base, ca, "PUT", f"/_matrix/client/v3/rooms/{r['room_id']}/send/m.room.message/seed{i}",
                 {"msgtype": "m.text", "body": f"Messaggio iniziale {i} ({server})"}, a)
        out["servers"][server] = {"url": f"https://{host}:{port}", "users": {
            f"@gnobbi-test-{u}:{NAME}": {"password": pw[u], "token": acc[u]["access_token"]} for u in pw}, "rooms": rooms}
    (RUN / "fixture.json").write_text(json.dumps(out, indent=1))
    (RUN / "fixture.json").chmod(0o600)
    print(RUN / "fixture.json")


def down():
    for n in ("proxy", "synapse", "tuwunel"):
        sh("docker", "rm", "-f", f"{NET}-{n}", check=False)
    sh("docker", "network", "rm", NET, check=False)
    shutil.rmtree(RUN, ignore_errors=True)


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("cmd", choices=("up", "down"))
    p.add_argument("--host", default=None, help="address the simulator/emulator uses to reach this machine (default: LAN IP)")
    a = p.parse_args()
    down() if a.cmd == "down" else up(a.host or lan_ip())
