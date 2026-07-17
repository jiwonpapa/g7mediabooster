#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e"
CONTAINER="g7mb-server-install-$$"
ACCESS_KEY="g7mbinstallaccess"
SECRET_KEY="g7mbinstallsecret0123456789"

if [[ "${CI:-}" != "true" ]]; then
    echo "server install smoke is destructive and may run only on an isolated CI host" >&2
    exit 2
fi
TEMP="$(mktemp -d)"

cleanup() {
    sudo systemctl disable --now g7mediabooster.target >/dev/null 2>&1 || true
    sudo rm -f \
        /etc/systemd/system/g7mediabooster.target \
        /etc/systemd/system/g7mediabooster-api.service \
        /etc/systemd/system/g7mediabooster-worker.service \
        /etc/systemd/system/g7mediabooster-cleanup.service \
        /etc/systemd/system/g7mediabooster-cleanup.timer \
        /etc/systemd/system/g7mediabooster-inventory.service \
        /etc/systemd/system/g7mediabooster-inventory.timer \
        /etc/systemd/system/g7mediabooster-backup.service \
        /etc/systemd/system/g7mediabooster-backup.timer
    sudo systemctl daemon-reload >/dev/null 2>&1 || true
    sudo rm -f \
        /usr/local/bin/g7mbctl \
        /usr/local/bin/g7mb-api \
        /usr/local/bin/g7mb-worker \
        /usr/local/libexec/g7mb-sandbox
    sudo rm -rf \
        /usr/local/share/g7mediabooster \
        /etc/g7mediabooster \
        /var/lib/g7mediabooster
    sudo userdel g7mediabooster >/dev/null 2>&1 || true
    sudo groupdel g7mediabooster >/dev/null 2>&1 || true
    docker rm --force "$CONTAINER" >/dev/null 2>&1 || true
    rm -rf -- "$TEMP"
}
trap cleanup EXIT

archive="$(find "$ROOT/output/releases" -maxdepth 1 -type f \
    -name 'g7mediabooster-server-*-linux-*.tar.gz' -print -quit)"
if [[ -z "$archive" ]]; then
    echo "server bundle was not found" >&2
    exit 2
fi
tar -xzf "$archive" -C "$TEMP"
bundle="$(find "$TEMP" -mindepth 1 -maxdepth 1 -type d -print -quit)"
if [[ -z "$bundle" ]]; then
    echo "extracted server bundle root was not found" >&2
    exit 2
fi

docker run --detach --rm \
    --name "$CONTAINER" \
    --env "MINIO_ROOT_USER=$ACCESS_KEY" \
    --env "MINIO_ROOT_PASSWORD=$SECRET_KEY" \
    --publish 127.0.0.1::9000 \
    "$IMAGE" server /data >/dev/null
port="$(docker port "$CONTAINER" 9000/tcp | sed -n '1s/.*://p')"
if [[ -z "$port" ]]; then
    echo "MinIO published port was not found" >&2
    exit 1
fi
endpoint="http://127.0.0.1:$port"
ready=false
for _ in $(seq 1 120); do
    if curl --fail --silent "$endpoint/minio/health/live" >/dev/null; then
        ready=true
        break
    fi
    sleep 0.25
done
if [[ "$ready" != true ]]; then
    docker logs "$CONTAINER" >&2
    exit 1
fi

printf '%s' "$ACCESS_KEY" >"$TEMP/access-key"
printf '%s' "$SECRET_KEY" >"$TEMP/secret-key"
chmod 0600 "$TEMP/access-key" "$TEMP/secret-key"

sudo "$bundle/bin/g7mbctl" install \
    --bundle-dir "$bundle" \
    --install-dependencies \
    --skip-setup \
    --skip-start
# The pinned MinIO protocol fixture does not implement PutBucketCors. Exact browser
# CORS remains covered by the provider-gated live conformance harness.
sudo /usr/local/bin/g7mbctl setup \
    --non-interactive \
    --provider generic \
    --endpoint-url "$endpoint" \
    --region us-east-1 \
    --bucket g7mb-install-private \
    --access-key-id-file "$TEMP/access-key" \
    --secret-access-key-file "$TEMP/secret-key" \
    --tenant-id install-smoke \
    --create-buckets \
    --skip-cors \
    --force-path-style
sudo systemctl enable --now g7mediabooster.target
api_ready=false
for _ in $(seq 1 120); do
    if curl --fail --silent http://127.0.0.1:8088/health/ready >/dev/null; then
        api_ready=true
        break
    fi
    sleep 0.25
done
if [[ "$api_ready" != true ]]; then
    sudo systemctl status \
        g7mediabooster.target \
        g7mediabooster-api.service \
        g7mediabooster-worker.service \
        --no-pager || true
    sudo journalctl \
        --unit g7mediabooster-api.service \
        --unit g7mediabooster-worker.service \
        --lines 200 \
        --no-pager || true
    exit 1
fi
sudo /usr/local/bin/g7mbctl status
sudo /usr/local/bin/g7mbctl doctor
sudo test -s \
    /usr/local/share/g7mediabooster/gnuboard7/jiwonpapa-g7mediabooster.zip

printf 'server-install-smoke PASS target=active api=ready worker=active timers=3 storage=single+multipart\n'
