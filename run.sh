#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"

profile=debug
build_only=false
show_args=()
build_args=(build --workspace --locked)
for argument in "$@"; do
    case "$argument" in
        --release) profile=release; build_args+=(--release) ;;
        --build-only) build_only=true ;;
        --show) show_args=(--show) ;;
        *) echo "Usage: bash run.sh [--release] [--build-only] [--show]" >&2; exit 1 ;;
    esac
done

cargo "${build_args[@]}"
mkdir -p "target/$profile/licenses"
cp ui/assets/icons/LICENSE "target/$profile/licenses/Lucide.txt"
if [[ ! -e "target/$profile/config.json" ]]; then
    cp config.example.json "target/$profile/config.json"
fi
for plugin in calculator applications clipboard chrome-bookmark encode decode formatter process-killer uuid-generator translator quick-text; do
    directory="target/$profile/plugins/$plugin"
    mkdir -p "$directory"
    cp "plugins/$plugin/plugin.json" "$directory/"
    mkdir -p "$directory/locales"
    cp plugins/"$plugin"/locales/*.json "$directory/locales/"
    cp "target/$profile/mytools-$plugin" "$directory/"
    if [[ -f "plugins/$plugin/config.example.json" ]]; then
        cp "plugins/$plugin/config.example.json" "$directory/"
    fi
done
if ! "$build_only"; then
    exec "target/$profile/mytools" "${show_args[@]}"
fi
