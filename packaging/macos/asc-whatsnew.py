#!/usr/bin/env python3
"""Imposta le note «What to Test» (betaBuildLocalizations.whatsNew) di una build TestFlight.

Chiamato da packaging/macos/upload-testflight.sh dopo un upload altool riuscito:
stessa chiave ASC già usata da altool (ASC_KEY_ID / ASC_ISSUER_ID / ASC_KEY_FILE
da ~/.config/gnobbi-ci/asc.env), nessun nuovo segreto né chiave.

Le note sono i commit visibili (crates/matrix-desktop/ o crates/matrix-domain/)
dal marker --state (SHA dell'ultimo upload riuscito, scritto dallo script shell);
se manca o non è raggiungibile, gli ultimi 15. Troncato a 4000 caratteri (limite ASC).

Uso: asc-whatsnew.py --app 6826654528 --build 42 --state ~/.config/gnobbi-ci/last-upload-sha
      [--notes-file F] [--dry-run]   (altrimenti le note vengono da git log)
Env: ASC_WHATSNEW_TIMEOUT secondi di attesa che la build compaia su ASC (default 300);
     ASC_API_BASE per i test (default https://api.appstoreconnect.apple.com).
Esce non-zero in caso di errore: il chiamante lo tratta come best-effort (warning CI).
"""
import argparse
import base64
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request

UI_PATHS = ("crates/matrix-desktop", "crates/matrix-domain")
WHATS_NEW_MAX = 4000


def _b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=")


def _der_to_raw(der):
    # ECDSA DER -> r||s (32+32 byte) richiesto da JWT ES256; strip degli 0x00 iniziali
    assert der[0] == 0x30
    i = 2 if der[1] < 0x80 else 3
    assert der[i] == 0x02
    rl = der[i + 1]
    r = der[i + 2 : i + 2 + rl]
    j = i + 2 + rl
    assert der[j] == 0x02
    sl = der[j + 1]
    s = der[j + 2 : j + 2 + sl]
    return r.lstrip(b"\0").rjust(32, b"\0") + s.lstrip(b"\0").rjust(32, b"\0")


def token():
    hdr = _b64(json.dumps({"alg": "ES256", "kid": os.environ["ASC_KEY_ID"], "typ": "JWT"}).encode())
    now = int(time.time())
    pl = _b64(json.dumps({"iss": os.environ["ASC_ISSUER_ID"], "iat": now, "exp": now + 900,
                          "aud": "appstoreconnect-v1"}).encode())
    msg = hdr + b"." + pl
    der = subprocess.run(["openssl", "dgst", "-sha256", "-sign", os.environ["ASC_KEY_FILE"]],
                         input=msg, capture_output=True, check=True).stdout
    return (msg + b"." + _b64(_der_to_raw(der))).decode()


def api(method, path, body=None):
    req = urllib.request.Request(os.environ.get("ASC_API_BASE", "https://api.appstoreconnect.apple.com") + path,
                                 method=method,
                                 data=json.dumps(body).encode() if body is not None else None,
                                 headers={"Authorization": "Bearer " + token(), "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            raw = r.read()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"ASC {method} {path[:60]}: HTTP {e.code} {e.read()[:200]!r}") from None


def _git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True).stdout.strip()


def notes_from_git(state_path):
    last = ""
    try:
        last = open(state_path).read().strip()
    except OSError:
        pass
    if last and subprocess.run(["git", "cat-file", "-e", last + "^{commit}"],
                               capture_output=True).returncode == 0:
        out = _git("log", "--oneline", f"{last}..HEAD", "--", *UI_PATHS)
        if out:
            return out, f"dall'ultimo upload {last[:9]}"
        return _git("log", "-15", "--oneline", "--", *UI_PATHS), f"nessun commit UI/dominio dopo {last[:9]}: ultimi 15"
    return _git("log", "-15", "--oneline", "--", *UI_PATHS), "ultimi 15 commit (marker assente o irraggiungibile)"


def truncate(text, limit=WHATS_NEW_MAX):
    if len(text) <= limit:
        return text
    cut = text[:limit]
    nl = cut.rfind("\n")
    return cut[:nl].rstrip() if nl > 0 else cut.rstrip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", required=True)
    ap.add_argument("--build", required=True)
    ap.add_argument("--state", help="file con lo SHA dell'ultimo upload riuscito")
    ap.add_argument("--notes-file", help="note già pronte (salta git log)")
    ap.add_argument("--dry-run", action="store_true", help="stampa le note e non tocca ASC")
    args = ap.parse_args()

    if args.notes_file:
        notes = open(args.notes_file).read().strip()
        how = args.notes_file
    else:
        notes, how = notes_from_git(args.state or "/nonexistent")
    notes = truncate(notes)
    print(f"note TestFlight ({how}, {len(notes)} caratteri):")
    print(notes if notes else "(vuote)")
    if not notes or args.dry_run:
        return

    deadline = time.time() + int(os.environ.get("ASC_WHATSNEW_TIMEOUT", "300"))
    while True:
        r = api("GET", f"/v1/builds?filter[app]={args.app}&filter[version]={args.build}&limit=1")
        if r.get("data"):
            break
        if time.time() > deadline:
            raise RuntimeError(f"build {args.build} non visibile su ASC dopo il timeout")
        time.sleep(15)
    bid = r["data"][0]["id"]

    locs = api("GET", f"/v1/builds/{bid}/betaBuildLocalizations")["data"]
    loc = next((l for l in locs if l["attributes"].get("locale") == "en"), locs[0] if locs else None)
    if loc:
        api("PATCH", f"/v1/betaBuildLocalizations/{loc['id']}",
            {"data": {"type": "betaBuildLocalizations", "id": loc["id"],
                      "attributes": {"whatsNew": notes}}})
    else:
        api("POST", "/v1/betaBuildLocalizations",
            {"data": {"type": "betaBuildLocalizations",
                      "attributes": {"locale": "en", "whatsNew": notes},
                      "relationships": {"build": {"data": {"type": "builds", "id": bid}}}}})
    print(f"What to Test impostato sulla build {args.build} (locale {loc['attributes']['locale'] if loc else 'en'})")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        print(f"::warning::note TestFlight non impostate: {e}", file=sys.stderr)
        sys.exit(1)
