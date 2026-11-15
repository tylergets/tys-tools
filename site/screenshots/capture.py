"""Capture real ty* apps inside a disposable, networkless Xvfb desktop."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
TOOLS = ["tybat", "tynotif", "tyboard", "tykey", "tycapture", "tyshot", "tyspeak",
         "tylight", "tygesture", "typort", "tyci", "typk", "tylock", "tyquote"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apps", type=Path, required=True, help="built package containing bin/ (nix build .#default)")
    parser.add_argument("--output", type=Path, default=Path("site/public/screenshots"))
    parser.add_argument("--only", choices=TOOLS, action="append", help="capture only these apps")
    parser.add_argument("--logs", type=Path, default=Path(tempfile.gettempdir()) / "tys-tools-screenshot-logs")
    args = parser.parse_args()
    apps = args.apps.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    logs = args.logs.resolve()
    logs.mkdir(parents=True, exist_ok=True)
    selected = args.only or TOOLS
    failures = []
    for tool in selected:
        if not (apps / "bin" / tool).is_file():
            parser.error(f"missing {apps / 'bin' / tool}")
    for tool in selected:
        print(f"Capturing {tool}…", flush=True)
        with tempfile.TemporaryDirectory(prefix=f"ty-screenshot-{tool}-") as temp:
            root = Path(temp)
            home = root / "home"
            home.mkdir()
            etc = root / "etc"
            (etc / "pam.d").mkdir(parents=True)
            (etc / "pam.d/tylock").write_text("auth required pam_deny.so\n")
            (etc / "passwd").write_text(f"demo:x:{os.getuid()}:{os.getgid()}:Demo:/home/demo:/bin/sh\n")
            (etc / "group").write_text(f"demo:x:{os.getgid()}:\n")
            (etc / "fonts").mkdir()
            (etc / "fonts/fonts.conf").write_text(f'''<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig><dir>{os.environ['SCREENSHOT_FONT_DIR']}</dir><cachedir>/home/demo/.cache/fontconfig</cachedir>
<alias><family>sans-serif</family><prefer><family>DejaVu Sans</family></prefer></alias>
<alias><family>monospace</family><prefer><family>DejaVu Sans Mono</family></prefer></alias></fontconfig>''')
            # No host HOME, sockets, hardware, network, processes, credentials, or bus.
            cmd = ["bwrap", "--unshare-all", "--die-with-parent", "--new-session",
                   "--ro-bind", "/nix/store", "/nix/store", "--proc", "/proc", "--dev", "/dev",
                   "--tmpfs", "/tmp", "--tmpfs", "/run", "--dir", "/sys",
                   "--ro-bind", str(etc), "/etc", "--bind", str(home), "/home/demo",
                   "--ro-bind", str(HERE), "/capture", "--ro-bind", str(apps), "/apps",
                   "--bind", str(output), "/output", "--dir", "/bin",
                   "--symlink", str(Path(shutil.which("sh")).resolve()), "/bin/sh",
                   "--clearenv"]
            env = {
                "PATH": os.environ["PATH"], "HOME": "/home/demo", "USER": "demo", "LOGNAME": "demo",
                "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8", "TZ": "UTC",
                "DISPLAY": ":99", "GDK_BACKEND": "x11", "XDG_SESSION_TYPE": "x11",
                "XDG_SESSION_ID": "screenshot", "XDG_RUNTIME_DIR": "/run/user",
                "XDG_CONFIG_HOME": "/home/demo/.config", "XDG_CACHE_HOME": "/home/demo/.cache",
                "XDG_STATE_HOME": "/home/demo/.local/state", "XDG_DATA_HOME": "/home/demo/.local/share",
                "GTK_THEME": "Adwaita:dark", "FONTCONFIG_FILE": "/etc/fonts/fonts.conf",
                "NO_AT_BRIDGE": "1",
                "SCREENSHOT_ICON_DIR": os.environ["SCREENSHOT_ICON_DIR"],
            }
            for key in ("GI_TYPELIB_PATH", "XDG_DATA_DIRS", "GIO_EXTRA_MODULES", "LD_LIBRARY_PATH", "LOCALE_ARCHIVE", "PYTHONPATH"):
                if key in os.environ:
                    env[key] = os.environ[key]
            for key, value in env.items():
                cmd += ["--setenv", key, value]
            cmd += ["--chdir", "/home/demo", "dbus-run-session", "--config-file=/capture/session.conf", "--", sys.executable, "/capture/scene.py", tool]
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=90)
            (logs / f"{tool}.log").write_text(result.stdout + result.stderr)
            if result.returncode:
                failures.append(tool)
                print(f"{tool} failed; see {logs / (tool + '.log')}\n{result.stderr[-2000:]}", file=sys.stderr)
    manifest_path = output / "manifest.json"
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
    for tool in TOOLS:
        image = output / f"{tool}.png"
        metadata = output / f"{tool}.json"
        if image.exists() and metadata.exists():
            manifest[tool] = {**json.loads(metadata.read_text()), "sha256": hashlib.sha256(image.read_bytes()).hexdigest()}
            metadata.unlink()
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    if failures:
        raise SystemExit("Failed scenes: " + ", ".join(failures))
    print(f"Saved {len(selected)} screenshots to {output}")


if __name__ == "__main__":
    main()
