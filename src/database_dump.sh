# Runs on the Docker host, under the SSH user. Never return dump bytes or credentials.
set -eu
set +x
umask 077
op=$1; stage=$2; prefix=$3; repo=$4; password=$5; count=$6
shift 6
case "$stage" in /*/containersafe-db-????????-????-????-????-????????????) ;; *) exit 80;; esac
[ "$count" -gt 0 ] && [ "$count" -le 128 ] || exit 80
case "$prefix" in
    'sudo -n '*) docker=${prefix#sudo -n }; use_sudo=1;;
    *) docker=$prefix; use_sudo=0;;
esac
case "$docker" in docker|/usr/bin/docker|/usr/local/bin/docker|/var/packages/ContainerManager/target/usr/bin/docker|/var/packages/Docker/target/usr/bin/docker) ;; *) exit 80;; esac
docker_cli() { if [ "$use_sudo" = 1 ]; then sudo -n "$docker" "$@"; else "$docker" "$@"; fi; }
# Cleanup only the exclusively owned directory, never recursively follow/remove paths.
cleanup() {
    [ -d "$stage" ] && [ ! -L "$stage" ] || return 1
    [ "$(stat -c %u "$stage")" = "$(id -u)" ] && [ "$(stat -c %a "$stage")" = 700 ] || return 1
    rm -f -- "$stage"/*.dump "$stage/manifest.json" || return 1
    rmdir -- "$stage"
}
if [ "$op" = cleanup ]; then cleanup || exit 84; exit 0; fi
[ "$op" = prepare ] || exit 80
parent=$(dirname "$stage")
[ -d "$parent" ] && [ ! -L "$parent" ] && [ -w "$parent" ] || exit 80
canonical=$(readlink -f "$parent") || exit 80
parent=$canonical
stage="$parent/$(basename "$stage")"
[ "$(stat -c %u "$parent")" = "$(id -u)" ] || exit 80
# Parent cannot be writable by other users (including the primary group).
case "$(stat -c %a "$parent")" in 700|500|755|750|711|710) ;; *) exit 80;; esac
case "$password/" in "$stage/"*) exit 80;; esac
case "$repo" in /*)
    repo=$(readlink -f "$repo") || exit 80
    case "$stage/" in "$repo/"*) exit 80;; esac
    case "$repo/" in "$stage/"*) exit 80;; esac;;
esac
# Validate every source before mkdir/export. Consume a copy of the positional list.
validate_sources() {
    n=$count
    while [ "$n" -gt 0 ]; do shift 4; n=$((n-1)); done
    for source in "$@"; do
        [ "$source" = "$stage" ] && continue
        source=$(readlink -f "$source") || exit 80
        [ "$source" = "$stage" ] && continue
        case "$stage/" in "$source/"*) exit 80;; esac
        case "$source/" in "$stage/"*) exit 80;; esac
    done
}
validate_sources "$@"
# Exclusive mkdir refuses stale dumps and overlapping attempts. Never reuse partial data.
mkdir -m 700 -- "$stage" || exit 81
trap 'cleanup || true' EXIT
trap 'exit 83' HUP INT TERM
n=0
printf '{"schema_version":1,"files":[' > "$stage/manifest.json"
while [ "$n" -lt "$count" ]; do
    cid=$1; engine=$2; sqlite=$3; name=$4; shift 4
    case "$name" in *[!a-zA-Z0-9_.-]*|'') exit 80;; esac
    case "$cid" in *[!a-fA-F0-9]*|'') exit 80;; esac
    [ "${#cid}" = 64 ] || exit 80
    # All scripts are constant. Container values are positional arguments, never eval.
    # In-container credentials stay there; stderr never reaches application logs.
    if ! docker_cli exec "$cid" sh -c '
set -eu
set +x
umask 077
engine=$1; sqlite=$2
read_secret() { [ -f "$1" ] && [ ! -L "$1" ] && [ -r "$1" ] || exit 82; cat -- "$1"; }
case "$engine" in
PostgreSQL)
    command -v pg_dumpall >/dev/null || exit 82
    if [ -n "${POSTGRES_PASSWORD_FILE:-}" ]; then PGPASSWORD=$(read_secret "$POSTGRES_PASSWORD_FILE");
    else PGPASSWORD=${PGPASSWORD:-${POSTGRES_PASSWORD:-}}; fi
    export PGPASSWORD
    exec pg_dumpall --host=127.0.0.1 --port=5432 --no-password --username="${PGUSER:-${POSTGRES_USER:-postgres}}";;
MariaDB|MySQL)
    if [ "$engine" = MariaDB ]; then client=mariadb-dump; else client=mysqldump; fi
    command -v "$client" >/dev/null || exit 82
    if [ -n "${MARIADB_ROOT_PASSWORD_FILE:-${MYSQL_ROOT_PASSWORD_FILE:-}}" ]; then
        MYSQL_PWD=$(read_secret "${MARIADB_ROOT_PASSWORD_FILE:-${MYSQL_ROOT_PASSWORD_FILE:-}}");
    else MYSQL_PWD=${MARIADB_ROOT_PASSWORD:-${MYSQL_ROOT_PASSWORD:-${MYSQL_PWD:-}}}; fi
    export MYSQL_PWD
    exec "$client" --host=127.0.0.1 --port=3306 --protocol=tcp --user=root --all-databases --lock-all-tables --quick --routines --events --triggers --hex-blob;;
Redis/Valkey)
    if command -v redis-cli >/dev/null; then client=redis-cli; else client=valkey-cli; fi
    command -v "$client" >/dev/null || exit 82
    if [ -n "${REDIS_PASSWORD_FILE:-}" ]; then REDISCLI_AUTH=$(read_secret "$REDIS_PASSWORD_FILE");
    else REDISCLI_AUTH=${REDISCLI_AUTH:-${REDIS_PASSWORD:-}}; fi
    if [ -n "$REDISCLI_AUTH" ]; then export REDISCLI_AUTH; else unset REDISCLI_AUTH; fi
    set --
    if [ -n "${CONTAINERSAFE_REDIS_USER:-}" ]; then set -- --user "$CONTAINERSAFE_REDIS_USER"; fi
    cluster=$("$client" "$@" --raw INFO cluster) || exit 82
    case "$cluster" in *cluster_enabled:0*) ;; *) exit 82;; esac
    tmp=$(mktemp -d /tmp/containersafe-db.XXXXXXXX) || exit 82
    trap '\''rm -f -- "$tmp/data.rdb"; rmdir -- "$tmp"'\'' EXIT
    "$client" "$@" --rdb "$tmp/data.rdb" >/dev/null || exit 82
    [ -s "$tmp/data.rdb" ] || exit 82
    cat -- "$tmp/data.rdb";;
MongoDB)
    command -v mongodump >/dev/null && command -v mongosh >/dev/null || exit 82
    mongosh --host 127.0.0.1 --port 27017 --quiet --eval '\''const h=db.hello(); if(!h.setName || h.msg === "isdbgrid") quit(1)'\'' >/dev/null || exit 82
    # Operator-provisioned mode-600 tool config avoids passwords in process args.
    config=${CONTAINERSAFE_MONGO_CONFIG:-}
    set -- --host=127.0.0.1 --port=27017 --archive --gzip --oplog --quiet
    if [ -n "$config" ]; then
        [ -f "$config" ] && [ ! -L "$config" ] && [ "$(stat -c %a "$config")" = 600 ] || exit 82
        [ "$(grep -c '\''^password:'\'' "$config")" = 1 ] || exit 82
        ! grep -Ev '\''^[[:space:]]*(#.*)?$|^password:[[:space:]]+.*$'\'' "$config" >/dev/null || exit 82
        [ -n "${MONGO_INITDB_ROOT_USERNAME:-}" ] || exit 82
        set -- "$@" --config="$config" --username="$MONGO_INITDB_ROOT_USERNAME" --authenticationDatabase=admin
    fi
    # Never silently drop --oplog: standalone and sharded instances are unsupported.
    exec mongodump "$@";;
SQLite)
    command -v sqlite3 >/dev/null || exit 82
    [ -f "$sqlite" ] && [ ! -L "$sqlite" ] && [ -r "$sqlite" ] || exit 82
    tmp=$(mktemp -d /tmp/containersafe-db.XXXXXXXX) || exit 82
    trap '\''rm -f -- "$tmp/data.sqlite"; rmdir -- "$tmp"'\'' EXIT
    sqlite3 -batch -bail -readonly "$sqlite" ".timeout 10000" ".backup $tmp/data.sqlite" || exit 82
    [ "$(sqlite3 -batch -bail -readonly "$tmp/data.sqlite" "PRAGMA quick_check;")" = ok ] || exit 82
    cat -- "$tmp/data.sqlite";;
*) exit 82;;
esac
' sh "$engine" "$sqlite" > "$stage/$cid.dump" 2>/dev/null; then
        case "$engine" in PostgreSQL) exit 85;; MariaDB) exit 86;; MySQL) exit 87;; Redis/Valkey) exit 88;; MongoDB) exit 89;; SQLite) exit 90;; *) exit 82;; esac
    fi
    [ -s "$stage/$cid.dump" ] || exit 82
    [ "$n" = 0 ] || printf ',' >> "$stage/manifest.json"
    printf '{"file":"%s.dump","engine":"%s","container":"%s"}' "$cid" "$engine" "$name" >> "$stage/manifest.json"
    n=$((n+1))
done
printf ']}\n' >> "$stage/manifest.json"
trap - EXIT HUP INT TERM
