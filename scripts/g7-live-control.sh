#!/usr/bin/env bash
set -euo pipefail

SSH_HOST="${G7MB_SSH_HOST:-g7devops}"
APP_ROOT="${G7MB_G7_ROOT:-/home/g7devops/public_html}"
APP_USER="${G7MB_G7_USER:-g7devops}"
MODULE_ID="jiwonpapa-g7mediabooster"
CONFIRM=""
DEPLOYMENT_ID=""

usage() {
    cat <<'EOF'
usage: scripts/g7-live-control.sh <preflight|status|apply|disable|rollback> [options]

Defaults:
  SSH host : g7devops
  G7 root  : /home/g7devops/public_html
  G7 user  : g7devops

Mutating commands require --confirm with the exact SSH host. `disable` keeps
configuration, credentials, database rows, and media objects for instant re-enable.
`rollback` additionally requires --deployment-id and restores the recorded G7 files.
EOF
}

if [[ $# -lt 1 ]]; then
    usage >&2
    exit 64
fi
ACTION="$1"
shift

while [[ $# -gt 0 ]]; do
    case "$1" in
        --confirm)
            [[ $# -ge 2 ]] || { echo "--confirm requires a value" >&2; exit 64; }
            CONFIRM="$2"
            shift 2
            ;;
        --deployment-id)
            [[ $# -ge 2 ]] || { echo "--deployment-id requires a value" >&2; exit 64; }
            DEPLOYMENT_ID="$2"
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 64
            ;;
    esac
done

case "$ACTION" in
    preflight|status) ;;
    apply|disable|rollback)
        if [[ "$CONFIRM" != "$SSH_HOST" ]]; then
            echo "refusing live mutation: use --confirm $SSH_HOST" >&2
            exit 64
        fi
        if [[ "$ACTION" == "rollback" \
            && ! "$DEPLOYMENT_ID" =~ ^[A-Za-z0-9._-]+$ ]]; then
            echo "rollback requires a safe --deployment-id" >&2
            exit 64
        fi
        ;;
    *)
        echo "unknown action: $ACTION" >&2
        usage >&2
        exit 64
        ;;
esac

if [[ ! "$SSH_HOST" =~ ^[A-Za-z0-9._-]+$ \
    || ! "$APP_USER" =~ ^[A-Za-z_][A-Za-z0-9_-]*$ \
    || ! "$APP_ROOT" =~ ^/[A-Za-z0-9._/-]+$ \
    || "$APP_ROOT" == *"/../"* \
    || "$APP_ROOT" == *"/.." ]]; then
    echo "unsafe SSH host, app user, or app root" >&2
    exit 64
fi
REMOTE_DEPLOYMENT_ID="${DEPLOYMENT_ID:--}"

ssh \
    -o BatchMode=yes \
    -o ConnectTimeout=10 \
    "$SSH_HOST" \
    bash -s -- "$ACTION" "$APP_ROOT" "$APP_USER" "$MODULE_ID" "$REMOTE_DEPLOYMENT_ID" <<'REMOTE'
set -euo pipefail

action="$1"
app_root="$2"
app_user="$3"
module_id="$4"
deployment_id="$5"

artisan() {
    sudo -u "$app_user" -H php "$app_root/artisan" "$@"
}

module_row() {
    artisan module:list --no-ansi 2>/dev/null | grep -F "$module_id" || true
}

module_is_active() {
    artisan module:list --status=active --no-ansi 2>/dev/null | grep -Fq "$module_id"
}

service_state() {
    systemctl is-active g7mediabooster.target 2>/dev/null || true
}

require_base() {
    [[ -f "$app_root/artisan" ]] || { echo "FAIL g7-root=$app_root" >&2; exit 1; }
    id "$app_user" >/dev/null 2>&1 || { echo "FAIL g7-user=$app_user" >&2; exit 1; }
    sudo -n true >/dev/null 2>&1 || { echo "FAIL passwordless-sudo=required" >&2; exit 1; }
    command -v php >/dev/null 2>&1 || { echo "FAIL php=missing" >&2; exit 1; }
}

print_status() {
    local row state ready="unavailable"
    state="$(service_state)"
    row="$(module_row)"
    if curl --fail --silent --show-error --max-time 2 \
        http://127.0.0.1:8088/health/ready >/dev/null 2>&1; then
        ready="ready"
    fi
    [[ -n "$row" ]] || row="not-installed"
    printf 'STATUS service=%s api=%s module=%s\n' "$state" "$ready" "$row"
}

require_base

case "$action" in
    preflight)
        echo "PASS ssh-host=$(hostname)"
        echo "PASS g7-root=$app_root user=$app_user php=$(php -r 'echo PHP_VERSION;')"
        artisan --version
        if command -v vips >/dev/null 2>&1; then
            echo "PASS libvips=$(vips --version | head -n 1)"
        else
            echo "PENDING libvips=missing installer-will-prompt"
        fi
        if command -v ffmpeg >/dev/null 2>&1 && command -v ffprobe >/dev/null 2>&1; then
            echo "PASS ffmpeg=installed ffprobe=installed"
        else
            echo "PENDING ffmpeg-or-ffprobe=missing installer-will-prompt"
        fi
        if [[ -x /usr/local/bin/g7mbctl ]]; then
            echo "PASS g7mbctl=installed"
        else
            echo "PENDING g7mbctl=not-installed"
        fi
        print_status
        ;;
    status)
        print_status
        ;;
    apply)
        [[ -x /usr/local/bin/g7mbctl ]] || {
            echo "FAIL g7mbctl is not installed" >&2
            exit 1
        }
        [[ -f /etc/g7mediabooster/g7mb.toml ]] || {
            echo "FAIL setup is incomplete: /etc/g7mediabooster/g7mb.toml" >&2
            exit 1
        }
        if [[ -z "$(module_row)" ]]; then
            echo "FAIL G7 module is not installed: $module_id" >&2
            exit 1
        fi
        sudo systemctl enable --now g7mediabooster.target
        if ! sudo /usr/local/bin/g7mbctl status; then
            sudo systemctl disable --now g7mediabooster.target >/dev/null 2>&1 || true
            echo "FAIL service readiness; target was stopped" >&2
            exit 1
        fi
        if ! module_is_active; then
            if ! artisan module:activate "$module_id"; then
                sudo systemctl disable --now g7mediabooster.target >/dev/null 2>&1 || true
                echo "FAIL module activation; target was stopped" >&2
                exit 1
            fi
        fi
        print_status
        echo "PASS applied host=$(hostname)"
        ;;
    disable)
        if module_is_active; then
            artisan module:deactivate "$module_id"
        fi
        sudo systemctl disable --now g7mediabooster.target >/dev/null 2>&1 || true
        if [[ "$(service_state)" == "active" ]]; then
            echo "FAIL target remains active" >&2
            exit 1
        fi
        print_status
        echo "PASS disabled data=preserved"
        ;;
    rollback)
        backup_root="/var/backups/g7mediabooster/$deployment_id"
        sudo test -f "$backup_root/receipt.txt" \
            && sudo test -d "$backup_root/files" \
            && sudo test -f "$backup_root/created-paths.txt" || {
            echo "FAIL rollback receipt is incomplete: $backup_root" >&2
            exit 1
        }
        sudo grep -Fxq "deployment_id=$deployment_id" "$backup_root/receipt.txt" || {
            echo "FAIL rollback receipt id mismatch" >&2
            exit 1
        }
        sudo grep -Fxq "app_root=$app_root" "$backup_root/receipt.txt" || {
            echo "FAIL rollback app root mismatch" >&2
            exit 1
        }
        sudo bash -c 'cd "$1" && sha256sum -c "$2"' \
            _ "$backup_root/files" "$backup_root/before.sha256" >/dev/null
        if module_is_active; then
            artisan module:deactivate "$module_id" --force
        fi
        if [[ -n "$(module_row)" ]]; then
            artisan module:uninstall "$module_id" --force
        fi
        sudo rm -rf -- \
            "$app_root/modules/_pending/$module_id" \
            "$app_root/modules/$module_id"
        while IFS= read -r relative; do
            [[ -n "$relative" ]] || continue
            if [[ ! "$relative" =~ ^[A-Za-z0-9._/-]+$ \
                || "$relative" == /* \
                || "$relative" == *"/../"* \
                || "$relative" == ../* ]]; then
                echo "FAIL unsafe rollback path: $relative" >&2
                exit 1
            fi
            sudo rm -f -- "$app_root/$relative"
        done < <(sudo cat "$backup_root/created-paths.txt")
        sudo rsync -a "$backup_root/files/" "$app_root/"
        artisan optimize:clear --no-ansi
        sudo systemctl reload php8.5-fpm.service
        sudo systemctl disable --now g7mediabooster.target >/dev/null 2>&1 || true
        print_status
        echo "PASS rollback deployment_id=$deployment_id module_data=preserved"
        ;;
esac
REMOTE
