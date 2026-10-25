set -eu
path=$1; size=$2; modified=$3; transfer=$4; artifact=$5; digest=$6
[ -f "$path" ] && [ ! -L "$path" ] && [ -r "$path" ] || exit 81
[ "$(stat -c %s "$path")" = "$size" ] && [ "$(stat -c %Y "$path")" = "$modified" ] || exit 81
printf 'CSAFE-STREAM/1\n%s\n%s\n%s\n%s\nrestic-repository-tar-v1\nencrypted\n\n' "$transfer" "$artifact" "$size" "$digest"
cat "$path" || exit 81
[ "$(stat -c %s "$path")" = "$size" ] && [ "$(stat -c %Y "$path")" = "$modified" ] || exit 81
printf '\nCSAFE-END/1 %s %s\n' "$transfer" "$digest"
