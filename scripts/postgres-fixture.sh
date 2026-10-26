#!/usr/bin/env bash
# Starts or stops a disposable PostgreSQL server for the integration tests in
# src-tauri/tests/postgres.rs.
#
#   scripts/postgres-fixture.sh start [major-version]   # default 17
#   scripts/postgres-fixture.sh stop
#
# Uses Docker when a daemon is available (any supported major version), otherwise the PostgreSQL
# binaries installed under /usr/lib/postgresql. TLS is on, with a throwaway CA whose server
# certificate names only `localhost`, so connecting to 127.0.0.1 exercises hostname checks.
#
# `start` writes target/postgres-fixture/env; load it with `source` before running:
#   cargo test -p aidb --test postgres -- --ignored
# Nothing here is a real secret: the server listens on loopback only and is deleted by `stop`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FIXTURE="$ROOT/target/postgres-fixture"
PORT="${AIDB_FIXTURE_PORT:-54329}"
PASSWORD="aidb-fixture-only"
CONTAINER="aidb-postgres-fixture"

have_docker() { command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; }

make_certs() {
  local dir="$1"
  mkdir -p "$dir"
  openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=aidb fixture CA" \
    -keyout "$dir/ca.key" -out "$dir/ca.crt" >/dev/null 2>&1
  openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" \
    -keyout "$dir/server.key" -out "$dir/server.csr" >/dev/null 2>&1
  printf 'subjectAltName=DNS:localhost\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n' \
    >"$dir/server.ext"
  openssl x509 -req -in "$dir/server.csr" -CA "$dir/ca.crt" -CAkey "$dir/ca.key" \
    -CAcreateserial -days 2 -extfile "$dir/server.ext" -out "$dir/server.crt" >/dev/null 2>&1
  chmod 600 "$dir/server.key"
}

wait_ready() {
  for _ in $(seq 1 60); do
    if PGPASSWORD="$PASSWORD" psql "host=localhost port=$PORT user=postgres dbname=postgres sslmode=disable" \
      -qtAc 'select 1' >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "PostgreSQL did not become ready" >&2
  return 1
}

start_docker() {
  local version="$1"
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  # The image refuses a key file owned by another user, so copy the certificates in as postgres.
  docker run -d --name "$CONTAINER" -p "127.0.0.1:$PORT:5432" \
    -e POSTGRES_PASSWORD="$PASSWORD" -v "$FIXTURE/certs:/certs-src:ro" \
    --entrypoint bash "postgres:$version" -c '
      mkdir -p /certs && cp /certs-src/server.crt /certs-src/server.key /certs/ &&
      chown postgres /certs/* && chmod 600 /certs/server.key &&
      exec docker-entrypoint.sh postgres -c ssl=on \
        -c ssl_cert_file=/certs/server.crt -c ssl_key_file=/certs/server.key' >/dev/null
}

start_local() {
  local bin
  bin="$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -1)"
  [ -n "$bin" ] || { echo "No Docker daemon and no PostgreSQL binaries found" >&2; exit 1; }
  local data
  data="$(mktemp -d "${TMPDIR:-/tmp}/aidb-pg-XXXXXX")"
  echo "$data" >"$FIXTURE/data-dir"
  cp "$FIXTURE/certs/server.crt" "$FIXTURE/certs/server.key" "$data/"
  local run=()
  # initdb refuses to run as root; use the postgres account the packages create.
  if [ "$(id -u)" -eq 0 ]; then
    chown -R postgres "$data"
    run=(runuser -u postgres --)
  fi
  printf '%s\n' "$PASSWORD" >"$data/pwfile"
  "${run[@]}" "$bin/initdb" -D "$data/db" -U postgres --pwfile="$data/pwfile" \
    --auth=scram-sha-256 >/dev/null
  "${run[@]}" "$bin/pg_ctl" -D "$data/db" -l "$data/log" -w start -o \
    "-p $PORT -c listen_addresses=localhost -c unix_socket_directories=$data -c ssl=on -c ssl_cert_file=$data/server.crt -c ssl_key_file=$data/server.key" \
    >/dev/null
  echo "$bin" >"$FIXTURE/bin-dir"
}

stop() {
  if have_docker; then docker rm -f "$CONTAINER" >/dev/null 2>&1 || true; fi
  if [ -f "$FIXTURE/data-dir" ]; then
    local data bin run=()
    data="$(cat "$FIXTURE/data-dir")"
    bin="$(cat "$FIXTURE/bin-dir" 2>/dev/null || true)"
    [ "$(id -u)" -eq 0 ] && run=(runuser -u postgres --)
    if [ -n "$bin" ] && [ -d "$data/db" ]; then
      "${run[@]}" "$bin/pg_ctl" -D "$data/db" -m immediate stop >/dev/null 2>&1 || true
    fi
    rm -rf "$data"
  fi
  rm -rf "$FIXTURE"
}

case "${1:-}" in
  start)
    version="${2:-17}"
    stop
    mkdir -p "$FIXTURE"
    make_certs "$FIXTURE/certs"
    if have_docker; then start_docker "$version"; else start_local; fi
    wait_ready
    cat >"$FIXTURE/env" <<EOF
export AIDB_TEST_POSTGRES_URL='postgres://postgres:$PASSWORD@localhost:$PORT/postgres'
export AIDB_TEST_POSTGRES_CA='$FIXTURE/certs/ca.crt'
# The driver must ignore these: the tests check that a profile without a password does not pick
# up PGPASSWORD and that a read-only session stays read-only despite PGOPTIONS.
export PGPASSWORD='$PASSWORD'
export PGOPTIONS='-c default_transaction_read_only=off'
EOF
    PGPASSWORD="$PASSWORD" psql "host=localhost port=$PORT user=postgres dbname=postgres sslmode=disable" \
      -qtAc 'show server_version'
    echo "Ready. Run: source $FIXTURE/env"
    ;;
  stop) stop ;;
  *)
    echo "usage: $0 start [major-version] | stop" >&2
    exit 2
    ;;
esac
