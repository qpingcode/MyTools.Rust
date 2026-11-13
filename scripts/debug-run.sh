#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")/.."

profile=debug
build_only=false
show_args=()
build_args=(build --workspace --locked)
for argument in "$@"; do
    case "$argument" in
        --release) profile=release; build_args+=(--release) ;;
        --build-only) build_only=true ;;
        --show) show_args=(--show) ;;
        *) echo "Usage: bash scripts/debug-run.sh [--release] [--build-only] [--show]" >&2; exit 1 ;;
    esac
done

cargo "${build_args[@]}"
mkdir -p "target/$profile/licenses"
cp ui/assets/icons/LICENSE "target/$profile/licenses/Lucide.txt"
# Its own data folder (config.json, window position, logs, plugin data), apart from the
# installed copy's.
export MYTOOLS_DATA_DIR="$PWD/target/$profile/data"
mkdir -p "$MYTOOLS_DATA_DIR"
if [[ ! -e "$MYTOOLS_DATA_DIR/config.json" ]]; then
    cp config.example.json "$MYTOOLS_DATA_DIR/config.json"
fi
for plugin in calculator applications clipboard chrome-bookmark encode decode formatter process-killer uuid-generator translator quick-text scripts-runner; do
    directory="target/$profile/plugins/$plugin"
    mkdir -p "$directory"
    cp "plugins/$plugin/plugin.json" "$directory/"
    mkdir -p "$directory/locales"
    cp plugins/"$plugin"/locales/*.json "$directory/locales/"
    cp "target/$profile/mytools-$plugin" "$directory/"
    for example in plugins/"$plugin"/*.example.json; do
        if [[ -f "$example" ]]; then
            cp "$example" "$directory/"
        fi
    done
done
if ! "$build_only"; then
    exec "target/$profile/mytools" "${show_args[@]}"
fi
