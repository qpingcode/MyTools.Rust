# Lucide icons

Selected SVG assets from [Lucide 1.66.0](https://github.com/lucide-icons/lucide/tree/1.66.0).
The upstream copyright and license notices are preserved in [LICENSE](LICENSE).

`names.txt` is the supported icon catalog. Names are referenced as `builtin:<name>`
and are part of the plugin contract; preserve existing names when updating upstream.

From PowerShell 7, run `./scripts/update-lucide.ps1` at the repository root to
download the pinned assets and regenerate `ui/icons.slint`. Normal builds do not
download anything. Slint embeds the SVG assets into the executable.
