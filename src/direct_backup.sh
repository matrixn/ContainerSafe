set -eu
umask 077
op=$1; exe=$2; password=$3; repo=$4; restores=$5; snapshot=$6; directory=$7
shift 7
canonical_source() {
    value=$1; suffix=
    while [ ! -e "$value" ] && [ ! -L "$value" ]; do
        name=$(basename "$value") || return 1
        case "$name" in .|..|/) return 1;; esac
        suffix="/$name$suffix"
        parent=$(dirname "$value") || return 1
        [ "$parent" != "$value" ] || return 1
        value=$parent
    done
    value=$(readlink -f "$value") || return 1
    printf '%s%s\n' "${value%/}" "$suffix"
}
for tool in stat readlink id; do command -v "$tool" >/dev/null 2>&1 || exit 76; done
[ -f "$exe" ] && [ -x "$exe" ] || exit 70
[ -f "$password" ] && [ ! -L "$password" ] && [ -r "$password" ] || exit 71
owner=$(stat -c %u "$password") || exit 71
mode=$(stat -c %a "$password") || exit 71
case "$mode" in 600|400) ;; *) exit 71;; esac
[ "$owner" = "$(id -u)" ] || [ "$owner" = 0 ] || exit 71
password=$(readlink -f "$password") || exit 71
local_repo=1
case "$repo" in sftp:*|s3:https://*) local_repo=0;; esac
if [ "$local_repo" = 1 ]; then
    if [ "$op" = init ]; then
        parent=$(readlink -f "$(dirname "$repo")") || exit 75
        [ -d "$parent" ] && [ -w "$parent" ] || exit 75
        if [ -e "$repo" ] || [ -L "$repo" ]; then
            [ -d "$repo" ] && [ ! -L "$repo" ] || exit 74
            entries=$(ls -A "$repo") || exit 75
            [ -z "$entries" ] || exit 74
        fi
        repo="$parent/$(basename "$repo")"
    else
        repo=$(readlink -f "$repo") || exit 75
        [ -d "$repo" ] && [ -r "$repo" ] || exit 75
    fi
fi
if [ "$local_repo" = 1 ]; then [ "$repo" != / ] || exit 72; fi
target=
if [ "$op" = restore ]; then
    restores=$(readlink -f "$restores") || exit 73
    [ -d "$restores" ] && [ -w "$restores" ] || exit 73
    [ "$restores" != / ] || exit 73
    target="$restores/$directory"
    [ ! -e "$target" ] && [ ! -L "$target" ] || exit 73
    if [ "$local_repo" = 1 ]; then
        case "$target/" in "$repo/"*) exit 73;; esac
        case "$repo/" in "$target/"*) exit 73;; esac
    fi
fi
for source in "$@"; do
    source=$(canonical_source "$source") || exit 72
    [ -n "$source" ] && [ "$source" != / ] || exit 72
    if [ "$op" = backup ]; then [ -e "$source" ] && [ -r "$source" ] || exit 72; fi
    case "$password/" in "$source/"*) exit 72;; esac
    if [ "$local_repo" = 1 ]; then
        case "$repo/" in "$source/"*) exit 72;; esac
        case "$source/" in "$repo/"*) exit 72;; esac
    fi
    if [ "$op" = restore ]; then
        case "$target/" in "$source/"*) exit 73;; esac
        case "$source/" in "$target/"*) exit 73;; esac
    fi
    shift
    set -- "$@" "$source"
done
# No inherited password commands, alternate repository, TLS or SFTP overrides.
unset RESTIC_PASSWORD RESTIC_PASSWORD_COMMAND RESTIC_REPOSITORY RESTIC_REPOSITORY_FILE RESTIC_HOST
RESTIC_PASSWORD_FILE=$password
export RESTIC_PASSWORD_FILE
case "$op" in
    backup) [ "$#" -gt 0 ] || exit 72; set -- backup -- "$@";;
    init) set -- init;;
    snapshots) set -- snapshots;;
    check) set -- check --read-data;;
    restore) mkdir "$target" || exit 73; set -- restore --target "$target" --overwrite never --verify "$snapshot";;
    *) exit 72;;
esac
case "$repo" in
    sftp:*) exec "$exe" --json --no-cache --repo "$repo" -o 'sftp.command=ssh -oBatchMode=yes -oStrictHostKeyChecking=yes' "$@";;
    *) exec "$exe" --json --no-cache --repo "$repo" "$@";;
esac
