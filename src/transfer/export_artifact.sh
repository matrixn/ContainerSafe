set -eu
umask 077
exe=$1; password=$2; source=$3; snapshot=$4; base=$5; artifact=$6
[ -x "$exe" ] || exit 70
[ -f "$password" ] && [ ! -L "$password" ] && [ -r "$password" ] || exit 71
case "$(stat -c %a "$password")" in 600|400) ;; *) exit 71;; esac
owner=$(stat -c %u "$password")
[ "$owner" = "$(id -u)" ] || [ "$owner" = 0 ] || exit 71
base=$(readlink -f "$base") || exit 73
[ -d "$base" ] && [ -w "$base" ] && [ "$base" != / ] || exit 73
root="$base/containersafe-encrypted-exports"
if [ ! -e "$root" ]; then mkdir -m 700 "$root" || exit 75; fi
[ -d "$root" ] && [ ! -L "$root" ] && [ "$(stat -c %u "$root")" = "$(id -u)" ] && [ "$(stat -c %a "$root")" = 700 ] || exit 75
final="$root/$artifact.csbackup.tar"
[ ! -e "$final" ] && [ ! -L "$final" ] || exit 74
work=$(mktemp -d "$root/.export-$artifact.XXXXXX") || exit 75
cleanup() { case "$work" in "$root/.export-$artifact."*) rm -rf -- "$work";; esac; }
trap cleanup EXIT
unset RESTIC_PASSWORD RESTIC_PASSWORD_COMMAND RESTIC_REPOSITORY RESTIC_REPOSITORY_FILE RESTIC_FROM_PASSWORD RESTIC_FROM_PASSWORD_COMMAND
RESTIC_PASSWORD_FILE=$password
export RESTIC_PASSWORD_FILE
set -- --json --no-cache --password-file "$password"
case "$source" in sftp:*) set -- "$@" -o 'sftp.command=ssh -oBatchMode=yes -oStrictHostKeyChecking=yes';; esac
"$exe" "$@" --repo "$work/repository" init >/dev/null 2>&1
"$exe" "$@" --repo "$work/repository" copy --from-repo "$source" --from-password-file "$password" "$snapshot" >/dev/null 2>&1
"$exe" "$@" --repo "$work/repository" check --read-data >/dev/null 2>&1
tar -C "$work" -cf "$work/artifact.tmp" repository
chmod 400 "$work/artifact.tmp"
ln "$work/artifact.tmp" "$final" || exit 74
digest=$(sha256sum "$final")
printf '%s\n' "${digest%% *}"
stat -c '%s' "$final"
stat -c '%Y' "$final"
printf '%s\n' "$final"
