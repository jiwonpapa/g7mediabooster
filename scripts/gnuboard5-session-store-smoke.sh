#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
module="$repo_root/adapters/gnuboard5/jiwonpapa-g7mediabooster"
image="${G7MB_G5_MYSQL_IMAGE:-mysql:8.4}"
container="g7mb-g5-mysql-$$"
password="g7mb-host-gate-password"

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

if [[ ! -f "$module/vendor/autoload.php" ]]; then
  composer install --working-dir="$module" --no-interaction --prefer-dist
fi

docker run --rm -d \
  --name "$container" \
  -e MYSQL_ROOT_PASSWORD="$password" \
  -e MYSQL_DATABASE=g7mb_test \
  -p 127.0.0.1::3306 \
  "$image" >/dev/null

ready=0
for _ in {1..60}; do
  if docker exec "$container" mysqladmin ping -h127.0.0.1 -uroot -p"$password" --silent >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
if [[ "$ready" != 1 ]]; then
  docker logs "$container" >&2
  exit 1
fi

port="$(docker port "$container" 3306/tcp | awk -F: 'NR == 1 { print $NF }')"
G7MB_G5_TEST_MYSQL_HOST=127.0.0.1 \
G7MB_G5_TEST_MYSQL_PORT="$port" \
G7MB_G5_TEST_MYSQL_DATABASE=g7mb_test \
G7MB_G5_TEST_MYSQL_USER=root \
G7MB_G5_TEST_MYSQL_PASSWORD="$password" \
php "$repo_root/scripts/gnuboard5-session-store-smoke.php"
