#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 6 ]]; then
    echo "usage: $0 APP_ROOT APP_USER MODULE_ZIP MODULE_SHA256 DEPLOYMENT_ID CONFIRM_ID" >&2
    exit 64
fi

APP_ROOT="$1"
APP_USER="$2"
MODULE_ZIP="$3"
MODULE_SHA256="$4"
DEPLOYMENT_ID="$5"
CONFIRM_ID="$6"
MODULE_ID="jiwonpapa-g7mediabooster"
MODULE_VERSION="0.4.3"
SCRIPT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PAYLOAD_ROOT="$(cd "$SCRIPT_ROOT/.." && pwd)"
PATCH_ROOT="$PAYLOAD_ROOT/adapters/gnuboard7/upstream-contract"
BACKUP_ROOT="/var/backups/g7mediabooster/$DEPLOYMENT_ID"
WORK=""
FILES_APPLIED=0
DEPLOYMENT_COMPLETE=0

if [[ "$(id -u)" != "0" ]]; then
    echo "remote module installer requires root" >&2
    exit 1
fi
if [[ "$CONFIRM_ID" != "$DEPLOYMENT_ID" ]]; then
    echo "deployment confirmation mismatch" >&2
    exit 64
fi
if [[ ! "$APP_ROOT" =~ ^/[A-Za-z0-9._/-]+$ \
    || ! "$APP_USER" =~ ^[A-Za-z_][A-Za-z0-9_-]*$ \
    || ! "$DEPLOYMENT_ID" =~ ^[A-Za-z0-9._-]+$ \
    || ! "$MODULE_SHA256" =~ ^[a-f0-9]{64}$ ]]; then
    echo "unsafe deployment argument" >&2
    exit 64
fi
if [[ ! -f "$APP_ROOT/artisan" || ! -d "$PATCH_ROOT" || ! -f "$MODULE_ZIP" ]]; then
    echo "G7 root, contract patches, or module ZIP is missing" >&2
    exit 1
fi
if [[ -e "$BACKUP_ROOT" ]]; then
    echo "backup receipt already exists: $BACKUP_ROOT" >&2
    exit 1
fi
if [[ "$(sha256sum "$MODULE_ZIP" | awk '{print $1}')" != "$MODULE_SHA256" ]]; then
    echo "module ZIP checksum mismatch" >&2
    exit 1
fi

artisan() {
    sudo -u "$APP_USER" -H php "$APP_ROOT/artisan" "$@"
}

rollback_on_error() {
    set +e
    if artisan module:list --status=active --no-ansi | grep -Fq "$MODULE_ID"; then
        artisan module:deactivate "$MODULE_ID" --force
    fi
    if artisan module:list --no-ansi | grep -Fq "$MODULE_ID"; then
        artisan module:uninstall "$MODULE_ID" --force
    fi
    rm -rf -- "$APP_ROOT/modules/_pending/$MODULE_ID" "$APP_ROOT/modules/$MODULE_ID"
    if [[ -f "$CREATED_PATHS" ]]; then
        while IFS= read -r relative; do
            rm -f -- "$APP_ROOT/$relative"
        done <"$CREATED_PATHS"
    fi
    if [[ -d "$BACKUP_ROOT/files" ]]; then
        rsync -a "$BACKUP_ROOT/files/" "$APP_ROOT/"
    fi
    artisan optimize:clear --no-ansi
    systemctl reload php8.5-fpm.service
}

finish() {
    local code=$?
    if [[ "$code" -ne 0 && "$FILES_APPLIED" -eq 1 \
        && "$DEPLOYMENT_COMPLETE" -eq 0 ]]; then
        echo "deployment failed; restoring module and G7 files" >&2
        rollback_on_error
    fi
    if [[ -n "$WORK" && -d "$WORK" ]]; then
        rm -rf -- "$WORK"
    fi
    exit "$code"
}
trap finish EXIT

WORK="$(mktemp -d /tmp/g7mb-live-install.XXXXXX)"
SHADOW="$WORK/host"
mkdir -p "$SHADOW/modules/_bundled" "$SHADOW/templates/_bundled"
chmod 0755 "$WORK" "$SHADOW"
cp -a "$APP_ROOT/app" "$SHADOW/app"
cp -a "$APP_ROOT/tests" "$SHADOW/tests"
cp -a "$APP_ROOT/modules/_bundled/sirsoft-board" \
    "$SHADOW/modules/_bundled/sirsoft-board"
cp -a "$APP_ROOT/templates/_bundled/sirsoft-basic" \
    "$SHADOW/templates/_bundled/sirsoft-basic"

patch_failures=0
patch_number=0
for patch_file in "$PATCH_ROOT"/*.patch; do
    patch_number=$((patch_number + 1))
    patch --batch --forward -p1 -d "$SHADOW" <"$patch_file" \
        >"$WORK/patch-$patch_number.log" 2>&1 \
        || patch_failures=$((patch_failures + 1))
done
rejects="$(
    find "$SHADOW" -type f -name '*.rej' -print \
        | sed "s#$SHADOW/##" \
        | sort
)"
expected_rejects="$(printf '%s\n' \
    modules/_bundled/sirsoft-board/composer.json.rej \
    modules/_bundled/sirsoft-board/module.json.rej \
    modules/_bundled/sirsoft-board/package-lock.json.rej \
    modules/_bundled/sirsoft-board/package.json.rej)"
if [[ "$patch_failures" -ne 1 || "$rejects" != "$expected_rejects" ]]; then
    echo "contract patch is not compatible with this live G7 revision" >&2
    printf '%s\n' "$rejects" >&2
    exit 1
fi
find "$SHADOW" -type f -name '*.rej' -delete

sudo -u "$APP_USER" -H \
    "$SCRIPT_ROOT/verify-gnuboard7-media-contract.sh" "$SHADOW"

PATCHED_PATHS="$WORK/patched-paths.txt"
TARGET_PATHS="$WORK/target-paths.txt"
CREATED_PATHS="$WORK/created-paths.txt"
grep -hE '^\+\+\+ b/' "$PATCH_ROOT"/*.patch \
    | sed 's#^+++ b/##' \
    | sort -u \
    | while IFS= read -r relative; do
        [[ -f "$SHADOW/$relative" ]] || continue
        if [[ ! -f "$APP_ROOT/$relative" ]] \
            || ! cmp -s "$SHADOW/$relative" "$APP_ROOT/$relative"; then
            printf '%s\n' "$relative"
        fi
    done >"$PATCHED_PATHS"
if [[ ! -s "$PATCHED_PATHS" ]]; then
    echo "no contract changes were produced" >&2
    exit 1
fi

while IFS= read -r relative; do
    printf '%s\n' "$relative"
    case "$relative" in
        modules/_bundled/sirsoft-board/*)
            active="modules/sirsoft-board/${relative#modules/_bundled/sirsoft-board/}"
            ;;
        templates/_bundled/sirsoft-basic/*)
            active="templates/sirsoft-basic/${relative#templates/_bundled/sirsoft-basic/}"
            ;;
        *)
            continue
            ;;
    esac
    if [[ -e "$APP_ROOT/$relative" && -e "$APP_ROOT/$active" ]] \
        && ! cmp -s "$APP_ROOT/$relative" "$APP_ROOT/$active"; then
        echo "active and bundled G7 files differ: $active" >&2
        exit 1
    fi
    if [[ -e "$APP_ROOT/$relative" && ! -e "$APP_ROOT/$active" ]] \
        || [[ ! -e "$APP_ROOT/$relative" && -e "$APP_ROOT/$active" ]]; then
        echo "active and bundled G7 file presence differs: $active" >&2
        exit 1
    fi
    printf '%s\n' "$active"
done <"$PATCHED_PATHS" | sort -u >"$TARGET_PATHS"

install -d -o root -g root -m 0700 "$BACKUP_ROOT/files"
touch "$CREATED_PATHS"
chmod 0600 "$CREATED_PATHS"
(
    cd "$APP_ROOT"
    while IFS= read -r relative; do
        if [[ -L "$relative" ]]; then
            echo "refusing symbolic-link deployment target: $relative" >&2
            exit 1
        fi
        if [[ -f "$relative" ]]; then
            cp --parents -a "$relative" "$BACKUP_ROOT/files"
        elif [[ ! -e "$relative" ]]; then
            printf '%s\n' "$relative" >>"$CREATED_PATHS"
        else
            echo "deployment target is not a regular file: $relative" >&2
            exit 1
        fi
    done <"$TARGET_PATHS"
)
(
    cd "$BACKUP_ROOT/files"
    find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum \
        >"$BACKUP_ROOT/before.sha256"
)
chmod 0600 "$BACKUP_ROOT/before.sha256"
cp "$TARGET_PATHS" "$BACKUP_ROOT/target-paths.txt"
cp "$PATCHED_PATHS" "$BACKUP_ROOT/patched-paths.txt"
cp "$CREATED_PATHS" "$BACKUP_ROOT/created-paths.txt"
printf '%s\n' \
    "deployment_id=$DEPLOYMENT_ID" \
    "module_id=$MODULE_ID" \
    "module_version=$MODULE_VERSION" \
    "module_zip_sha256=$MODULE_SHA256" \
    "app_root=$APP_ROOT" \
    "app_user=$APP_USER" \
    >"$BACKUP_ROOT/receipt.txt"
chmod 0600 "$BACKUP_ROOT"/*.txt

install_patched_file() {
    local source="$1"
    local destination="$2"
    local temporary="$destination.g7mb-new-$DEPLOYMENT_ID"
    install -d -o "$APP_USER" -g www-data -m 0755 "$(dirname "$destination")"
    install -o "$APP_USER" -g www-data -m 0644 "$source" "$temporary"
    mv -f "$temporary" "$destination"
}

FILES_APPLIED=1
while IFS= read -r relative; do
    source="$SHADOW/$relative"
    install_patched_file "$source" "$APP_ROOT/$relative"
    case "$relative" in
        modules/_bundled/sirsoft-board/*)
            active="modules/sirsoft-board/${relative#modules/_bundled/sirsoft-board/}"
            install_patched_file "$source" "$APP_ROOT/$active"
            ;;
        templates/_bundled/sirsoft-basic/*)
            active="templates/sirsoft-basic/${relative#templates/_bundled/sirsoft-basic/}"
            install_patched_file "$source" "$APP_ROOT/$active"
            ;;
    esac
done <"$PATCHED_PATHS"

sudo -u "$APP_USER" -H \
    "$SCRIPT_ROOT/verify-gnuboard7-media-contract.sh" "$APP_ROOT"
systemctl reload php8.5-fpm.service

sudo -u "$APP_USER" -H php \
    "$SCRIPT_ROOT/verify-gnuboard7-module-zip.php" \
    "$APP_ROOT" "$MODULE_ZIP" "$MODULE_VERSION"
if unzip -Z1 "$MODULE_ZIP" \
    | grep -E '(^/|(^|/)\.\.(/|$))' >/dev/null; then
    echo "unsafe module ZIP path" >&2
    exit 1
fi
MODULE_STAGE="$WORK/module-stage"
mkdir -p "$MODULE_STAGE"
unzip -q "$MODULE_ZIP" -d "$MODULE_STAGE"
if [[ ! -f "$MODULE_STAGE/$MODULE_ID/module.json" ]]; then
    echo "module ZIP root is invalid" >&2
    exit 1
fi
if artisan module:list --no-ansi | grep -Fq "$MODULE_ID"; then
    echo "module is already installed: $MODULE_ID" >&2
    exit 1
fi
if [[ -e "$APP_ROOT/modules/_pending/$MODULE_ID" \
    || -e "$APP_ROOT/modules/$MODULE_ID" ]]; then
    echo "module filesystem path already exists" >&2
    exit 1
fi
install -d -o "$APP_USER" -g www-data -m 0755 "$APP_ROOT/modules/_pending"
rsync -a --chown="$APP_USER:www-data" \
    "$MODULE_STAGE/$MODULE_ID/" "$APP_ROOT/modules/_pending/$MODULE_ID/"

if ! artisan module:install "$MODULE_ID" --vendor-mode=auto; then
    echo "module install failed" >&2
    exit 1
fi
if ! artisan module:activate "$MODULE_ID"; then
    echo "module activation failed" >&2
    exit 1
fi
artisan optimize:clear --no-ansi
systemctl reload php8.5-fpm.service

artisan module:list --no-ansi | grep -F "$MODULE_ID"
artisan route:list --no-ansi | grep -F "$MODULE_ID" | head -n 30 || true
DEPLOYMENT_COMPLETE=1
printf 'PASS live-module deployment_id=%s backup=%s module=%s version=%s\n' \
    "$DEPLOYMENT_ID" "$BACKUP_ROOT" "$MODULE_ID" "$MODULE_VERSION"
