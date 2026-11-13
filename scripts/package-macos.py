"""Build a macOS app bundle and DMG using only Python and Apple's bundled tools."""
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def run(*args):
    subprocess.run(args, cwd=ROOT, check=True)


def main():
    if platform.system() != "Darwin":
        raise SystemExit("DMG packaging requires macOS")
    version = os.environ.get("MYTOOLS_VERSION", "0.0.1").removeprefix("v")
    if not version or any(ch not in "0123456789." for ch in version):
        raise SystemExit("MYTOOLS_VERSION must be a numeric dotted version")
    run("bash", "scripts/debug-run.sh", "--release", "--build-only")
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    architecture = platform.machine()
    archive = dist / f"MyTools-macos-{architecture}.dmg"
    with tempfile.TemporaryDirectory(prefix="mytools-macos-") as temporary:
        stage = Path(temporary)
        app = stage / "MyTools.app"
        contents = app / "Contents"
        binaries = contents / "MacOS"
        resources = contents / "Resources"
        binaries.mkdir(parents=True)
        resources.mkdir()
        shutil.copy2(ROOT / "target/release/mytools", binaries / "mytools")
        for manifest in sorted((ROOT / "plugins").glob("*/plugin.json")):
            metadata = json.loads(manifest.read_text(encoding="utf-8-sig"))
            destination = binaries / "plugins" / manifest.parent.name
            destination.mkdir(parents=True)
            shutil.copy2(manifest, destination / "plugin.json")
            shutil.copy2(ROOT / "target/release" / metadata["entry"], destination / metadata["entry"])
            shutil.copytree(manifest.parent / "locales", destination / "locales")
            for example in manifest.parent.glob("*.example.json"):
                shutil.copy2(example, destination / example.name)
        shutil.copy2(ROOT / "README.md", resources / "README.md")
        shutil.copytree(ROOT / "docs", resources / "docs")
        (resources / "licenses").mkdir()
        shutil.copy2(ROOT / "ui/assets/icons/LICENSE", resources / "licenses/Lucide.txt")
        iconset = stage / "AppIcon.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                suffix = "@2x" if scale == 2 else ""
                run("sips", "-z", str(size * scale), str(size * scale), str(ROOT / "ui/assets/app.png"), "--out", str(iconset / f"icon_{size}x{size}{suffix}.png"))
        run("iconutil", "-c", "icns", str(iconset), "-o", str(resources / "AppIcon.icns"))
        # The iconset is a build input, not part of the distributed disk image.
        shutil.rmtree(iconset)
        with (contents / "Info.plist").open("wb") as output:
            plistlib.dump({
                "CFBundleName": "MyTools",
                "CFBundleDisplayName": "MyTools",
                "CFBundleIdentifier": "com.mytools.rust",
                "CFBundleExecutable": "mytools",
                "CFBundlePackageType": "APPL",
                "CFBundleInfoDictionaryVersion": "6.0",
                "CFBundleShortVersionString": version,
                "CFBundleVersion": version,
                "CFBundleIconFile": "AppIcon.icns",
                "NSHighResolutionCapable": True,
                "LSMinimumSystemVersion": "11.0",
            }, output)
        for executable in sorted((binaries / "plugins").glob("*/*")):
            if executable.is_file() and executable.name.startswith("mytools-"):
                run("codesign", "--force", "--sign", "-", str(executable))
        run("codesign", "--force", "--sign", "-", str(app))
        run("codesign", "--verify", "--deep", "--strict", str(app))
        (stage / "Applications").symlink_to("/Applications")
        run("hdiutil", "create", "-volname", "MyTools", "-srcfolder", str(stage), "-ov", "-format", "UDZO", str(archive))
        run("hdiutil", "verify", str(archive))
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix(".dmg.sha256").write_text(f"{checksum}  {archive.name}\n", encoding="ascii")
    print(f"Package: {archive}")


if __name__ == "__main__":
    main()
