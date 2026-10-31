#!/usr/bin/env bash
# Probe usa e getta della user directory (POST /_matrix/client/v3/user_directory/search)
# via matrix-sdk 0.19.1 (Client::search_users) su Synapse e Tuwunel in container.
#
# Uso:
#   ./run.sh build     # compila probe/ dentro un container rust:1 (cache nel volume userdir-cargo)
#   ./run.sh default   # scenario: Synapse e Tuwunel con configurazione di default
#   ./run.sh all       # scenario: search_all_users (Synapse) e show_all_local_users (Tuwunel)
#   ./run.sh live      # test live engine user_search_live su Synapse e Tuwunel (cargo sul host)
#   ./run.sh up        # server popolati e lasciati accesi (18008 Synapse, 18009 Tuwunel)
#   ./run.sh down      # spegne ed elimina i volumi
#
# Ogni scenario ricrea i server da zero (volumi freschi), registra
# alice/bob (stanza privata condivisa) e carol (nessuna stanza), poi cerca
# come alice e come carol su entrambi i server, incrociando anche termini
# dell'altra istanza. Output: out/<scenario>.jsonl (una riga JSON per evento).

set -euo pipefail
cd "$(dirname "$0")"

PROJ=userdir-live
NET="${PROJ}_default"
RUST_IMG=rust:1
SYN_URL=http://synapse:8008
TUW_URL=http://tuwunel:8008
SYN_NAME=synapse.test
TUW_NAME=tuwunel.test
BIN=/probe/target/release/userdir-probe

probe() { # probe <homeserver_url> <arg>...  → userdir-probe <arg0> <url> <resto>
  local url=$1; shift
  docker run --rm --network "$NET" -v "$PWD/probe:/probe" "$RUST_IMG" "$BIN" "$1" "$url" "${@:2}"
}

wait_up() {
  local i
  for i in $(seq 1 60); do
    curl -fsS "$1/_matrix/client/versions" >/dev/null 2>&1 && return 0
    sleep 1
  done
  echo "timeout in attesa di $1" >&2
  return 1
}

build() {
  # Monta la radice del repo: probe/ dipende via path dal matrix-sdk vendored.
  local repo
  repo=$(cd ../.. && pwd)
  docker run --rm -v "$repo:/repo" -w /repo/scripts/userdir-live/probe \
    -v userdir-cargo:/usr/local/cargo/registry \
    rust:1 bash -c 'apt-get update -qq >/dev/null && apt-get install -y -qq cmake clang >/dev/null 2>&1; cargo build --release'
}

down() {
  docker compose -p "$PROJ" down -v --remove-orphans
}

up() {
  local mode=$1
  {
    cat <<EOF
server_name = "$TUW_NAME"
database_path = "/var/lib/tuwunel"
address = ["0.0.0.0"]
port = 8008
allow_registration = true
yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse = true
new_user_displayname_suffix = ""
EOF
    [ "$mode" = all ] && echo 'show_all_local_users_in_user_directory = true'
  } > tuwunel-runtime.toml

  docker compose -p "$PROJ" up -d tuwunel

  # Prima generazione della config Synapse (volumi freschi), poi estensione.
  # Nota: homeserver.yaml generato non finisce con newline, si prepende una riga vuota.
  docker compose -p "$PROJ" run --rm synapse generate >/dev/null
  docker compose -p "$PROJ" run --rm --entrypoint bash synapse \
    -c 'cat >> /data/homeserver.yaml' <<EOF

enable_registration: true
enable_registration_without_verification: true
$([ "$mode" = all ] && printf 'user_directory:\n  search_all_users: true')
EOF
  docker compose -p "$PROJ" up -d synapse

  wait_up http://localhost:18008
  wait_up http://localhost:18009
}

scenario() {
  local mode=$1
  [ -x probe/target/release/userdir-probe ] || { echo "probe non compilato: ./run.sh build" >&2; exit 1; }
  down >/dev/null 2>&1 || true
  up "$mode"
  probe "$SYN_URL" seed "$SYN_NAME"
  probe "$TUW_URL" seed "$TUW_NAME"
  # attesa: la user directory di Synapse si popola con un background update
  # che può partire tardi e svuotare la FTS durante il ripopolamento; servono
  # due round consecutivi con bob visibile per considerarla stabile.
  stable=0
  for _ in $(seq 1 15); do
    syn=$(probe "$SYN_URL" ready "$SYN_NAME" | tail -1)
    tuw=$(probe "$TUW_URL" ready "$TUW_NAME" | tail -1)
    echo "ready syn=$syn tuw=$tuw" >&2
    if grep -q '"bob_visible":true' <<<"$syn" && grep -q '"bob_visible":true' <<<"$tuw"; then
      stable=$((stable + 1))
      [ "$stable" -ge 2 ] && break
    else
      stable=0
    fi
    sleep 3
  done
  [ "$mode" = up ] && return 0  # server pronti e popolati, restano accesi (QA a mano)
  if [ "$mode" = live ]; then
    # Test live del dialogo «Nuova chat» (ignorato di default): un giro per server.
    local rc=0 repo
    repo=$(cd ../.. && pwd)
    for pair in "http://localhost:18008 $SYN_NAME" "http://localhost:18009 $TUW_NAME"; do
      set -- $pair
      (cd "$repo" && USERDIR_URL=$1 USERDIR_SERVER=$2 cargo test -p matrix-engine --features sdk \
        --test user_search_live -- --ignored --nocapture) || rc=1
    done
    down
    return $rc
  fi
  mkdir -p out
  : > "out/$mode.jsonl"
  for local in alice carol; do
    probe "$SYN_URL" search "$SYN_NAME" "$local" "$TUW_NAME" | tee -a "out/$mode.jsonl"
    probe "$TUW_URL" search "$TUW_NAME" "$local" "$SYN_NAME" | tee -a "out/$mode.jsonl"
  done
  down
}

case "${1:-}" in
  build) build ;;
  default | all | live | up) scenario "$1" ;;
  down) down ;;
  *) echo "uso: $0 build|default|all|live|down" >&2; exit 2 ;;
esac
