#!/usr/bin/env bash
# Run only on an ephemeral CI runner; trust roots and services are job-local.
set -euo pipefail
[[ ${GITHUB_ACTIONS:-} == true ]] || { echo 'Run this fixture on GitHub Actions'; exit 1; }
fixture=$(mktemp -d "${RUNNER_TEMP}/crlt-remote.XXXXXX")
sqld_pid=
proxy_pid=
cleanup() {
    if [[ -n $proxy_pid ]]; then kill "$proxy_pid" || true; fi
    if [[ -n $sqld_pid ]]; then kill "$sqld_pid" || true; fi
}
trap cleanup EXIT

asset=libsql-server-x86_64-unknown-linux-gnu.tar.xz
curl --fail --silent --show-error --location --user-agent crlt-ci \
    "https://github.com/tursodatabase/libsql/releases/download/libsql-server-v0.24.32/$asset" \
    --output "$fixture/$asset"
printf '71720fc8648c19efef416efebd47145ef59b62e198770533530a858e1336879f  %s\n' "$fixture/$asset" | sha256sum --check
tar -xf "$fixture/$asset" -C "$fixture"

openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=crlt-test-ca \
    -keyout "$fixture/ca.key" -out "$fixture/ca.crt" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj /CN=localhost \
    -keyout "$fixture/server.key" -out "$fixture/server.csr" 2>/dev/null
printf '%s\n' 'subjectAltName=DNS:localhost' 'basicConstraints=CA:FALSE' \
    'keyUsage=digitalSignature,keyEncipherment' 'extendedKeyUsage=serverAuth' > "$fixture/server.ext"
openssl x509 -req -in "$fixture/server.csr" -CA "$fixture/ca.crt" -CAkey "$fixture/ca.key" \
    -CAcreateserial -days 1 -extfile "$fixture/server.ext" -out "$fixture/server.crt" 2>/dev/null
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=localhost \
    -addext subjectAltName=DNS:localhost -keyout "$fixture/untrusted.key" \
    -out "$fixture/untrusted.crt" 2>/dev/null
# Job-scoped trust bundle (no host CA mutation): the fixture CA is exposed
# only to this job via the standard bundle variables consumed by the actual
# TLS clients — rustls-native-certs (Rust reqwest `rustls` feature), curl /
# OpenSSL, Python requests and Node. Exact TLS assertions in `tests/remote.rs`
# (untrusted root on :18444, wrong hostname, redirect/downgrade/userinfo,
# missing-baton/status, http downgrade) are unchanged and still fail closed.
export SSL_CERT_FILE="$fixture/ca.crt"
export REQUESTS_CA_BUNDLE="$fixture/ca.crt"
export CURL_CA_BUNDLE="$fixture/ca.crt"
export NODE_EXTRA_CA_CERTS="$fixture/ca.crt"

# Disposable local JWT; no hosted database or external credentials are used.
openssl genpkey -algorithm Ed25519 -out "$fixture/jwt.key"
openssl pkey -in "$fixture/jwt.key" -pubout -out "$fixture/jwt.pub"
CRLT_TEST_TOKEN=$(python3 - "$fixture" <<'PY'
import base64, json, pathlib, subprocess, sys, time
fixture = pathlib.Path(sys.argv[1])
def encode(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=")
message = encode(b'{"alg":"EdDSA","typ":"JWT"}') + b"." + encode(json.dumps({"exp": int(time.time()) + 3600}).encode())
(fixture / "jwt.message").write_bytes(message)
signature = subprocess.check_output(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(fixture / "jwt.key"), "-in", str(fixture / "jwt.message")])
print((message + b"." + encode(signature)).decode())
PY
)
export CRLT_TEST_TOKEN
export CRLT_TEST_URL=https://localhost:18443
# Close the job lock FDs (8 toolchain, 9 state, owned by run-cached.sh) in each
# daemon child at launch so fixtures never retain the locks; cleanup stays
# exact-PID below, no broad pkill.
"$fixture/libsql-server-x86_64-unknown-linux-gnu/sqld" \
    --db-path "$fixture/data" --http-listen-addr 127.0.0.1:18080 \
    --http-self-url https://localhost:18443 \
    --auth-jwt-key-file "$fixture/jwt.pub" > "$fixture/sqld.log" 2>&1 8>&- 9>&- &
sqld_pid=$!
python3 .ci/tls_proxy.py "$fixture" > "$fixture/proxy.log" 2>&1 8>&- 9>&- &
proxy_pid=$!
for attempt in {1..50}; do
    if curl --silent --fail --user-agent crlt-ci http://127.0.0.1:18080/health >/dev/null \
        && curl --silent --fail --cacert "$fixture/ca.crt" --user-agent crlt-ci https://localhost:18443/ >/dev/null; then
        break
    fi
    kill -0 "$sqld_pid" "$proxy_pid" || { cat "$fixture/sqld.log" "$fixture/proxy.log"; exit 1; }
    sleep 0.2
done
if [[ ${1:-} == --coverage ]]; then
    cargo llvm-cov --locked --workspace --all-features --branch \
        --no-cfg-coverage --no-cfg-coverage-nightly --no-report \
        --test remote -- --ignored --test-threads=1
    cargo llvm-cov --locked --workspace --all-features --branch \
        --no-cfg-coverage --no-cfg-coverage-nightly --no-report \
        --lib -- --ignored --test-threads=1
else
    cargo test --locked --test remote -- --ignored --test-threads=1
    cargo test --locked --lib -- --ignored --test-threads=1
fi
