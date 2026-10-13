#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="g7mb-minio-test:7aac2a2c5b7c"
CONTAINER="g7mb-minio-$$"
ACCESS_KEY="g7mbtestaccess"
SECRET_KEY="g7mbtestsecret0123456789"

cleanup() {
    docker rm --force "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

command -v docker >/dev/null
command -v curl >/dev/null
docker info >/dev/null
docker build --quiet --file "$ROOT/tests/docker/minio.Dockerfile" --tag "$IMAGE" "$ROOT/tests/docker" >/dev/null

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

export G7MB_TEST_S3_ENDPOINT="$endpoint"
export G7MB_TEST_S3_ACCESS_KEY="$ACCESS_KEY"
export G7MB_TEST_S3_SECRET_KEY="$SECRET_KEY"

cd "$ROOT"
cargo test --locked --package g7mb-object-store-s3 \
    --test minio_conformance -- --ignored --nocapture
