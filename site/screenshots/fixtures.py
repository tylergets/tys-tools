"""Public, synthetic inputs supplied through the apps' normal interfaces."""
import os
from pathlib import Path
import sys


def prepare(tool):
    home = Path.home()
    config = home / ".config"
    (config / "gtk-3.0").mkdir(parents=True)
    (config / "gtk-3.0/settings.ini").write_text("[Settings]\ngtk-theme-name=Adwaita\ngtk-application-prefer-dark-theme=true\ngtk-font-name=DejaVu Sans 11\ngtk-enable-animations=false\n")
    (config / "openbox").mkdir()
    (config / "openbox/rc.xml").write_text('''<openbox_config xmlns="http://openbox.org/3.4/rc">
<theme><name>Onyx</name><titleLayout>NLIMC</titleLayout>
<font place="ActiveWindow"><name>DejaVu Sans</name><size>11</size><weight>Normal</weight></font>
<font place="InactiveWindow"><name>DejaVu Sans</name><size>11</size><weight>Normal</weight></font></theme>
<focus><focusNew>yes</focusNew></focus>
<placement><policy>Smart</policy><center>yes</center></placement>
</openbox_config>''')
    icons = home / ".local/share/icons"
    icons.mkdir(parents=True, exist_ok=True)
    for theme in Path(os.environ["SCREENSHOT_ICON_DIR"]).iterdir():
        if theme.is_dir() and theme.name != "hicolor":
            (icons / theme.name).symlink_to(theme)
    configs = {
        "tybat": "countdown_seconds: 60\naction: suspend\n",
        "tyboard": "clipboard: { active_window: { backend: { kind: disabled } } }\n",
        "tyspeak": "auto_download: false\n",
        "tykey": "expansions:\n  - { trigger: ':email', expansion: 'hello@example.com' }\n  - { trigger: ':thanks', expansion: 'Thanks for your help!' }\nsnippets:\n  - { title: Email address, content: 'hello@example.com' }\n  - { title: Meeting notes, content: 'Agenda / Decisions / Next steps' }\n  - { title: Project URL, content: 'https://example.com/project' }\n",
        "tylight": "location: { latitude: 39.10, longitude: -84.51 }\nambient_light: false\nmonitor_brightness: { backlight: false, ddc: false }\nfade_seconds: 0\n",
        "tygesture": "swipe:\n  3:\n    left: xdotool key alt+Right\n    right: xdotool key alt+Left\n    up: xdotool key super+w\npinch:\n  2: { in: 'xdotool key ctrl+minus', out: 'xdotool key ctrl+plus' }\n",
        "tyci": "repos: { include: [example] }\nnotifications: { on_finish: false }\npoll_seconds: 3600\n",
        "tylock": "fingerprint: false\ntime_format: '09:41'\ndate_format: 'Monday, November 16'\nquote_command: [printf, 'Make room for the work that matters.']\n",
        "typk": "helper: /home/demo/bin/polkit-helper\n",
        "tynotif": "timeout_ms: 0\n",
    }
    for app, content in configs.items():
        folder = config / app
        folder.mkdir(parents=True)
        (folder / "config.yaml").write_text(content + "appearance: { font: DejaVu Sans }\n")
    bindir = home / "bin"
    bindir.mkdir()
    gh = bindir / "gh"
    gh.write_text(f"#!{sys.executable}\n" + '''import json, sys
from datetime import datetime, timezone
now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
kind = sys.argv[1:3]
if kind == ["repo", "list"]:
    data = [{"nameWithOwner": "example/desktop", "isArchived": False}]
elif kind == ["search", "prs"]:
    data = [{"number": 42, "title": "Add keyboard navigation", "url": "https://example.com/pr/42", "repository": {"nameWithOwner": "example/desktop"}, "author": {"login": "demo"}, "createdAt": now, "isDraft": False}]
elif kind == ["run", "list"]:
    data = [{"databaseId": 1, "workflowName": "Build & test", "displayTitle": "Add keyboard navigation", "headBranch": "keyboard-navigation", "event": "push", "status": "in_progress", "conclusion": "", "url": "https://example.com/run/1", "startedAt": now, "updatedAt": now}, {"databaseId": 2, "workflowName": "Pages", "displayTitle": "Update documentation", "headBranch": "main", "event": "push", "status": "completed", "conclusion": "success", "url": "https://example.com/run/2", "startedAt": now, "updatedAt": now}]
else:
    raise SystemExit("Unexpected fixture gh invocation: " + repr(sys.argv))
print(json.dumps(data))
''')
    gh.chmod(0o755)
    helper = bindir / "polkit-helper"
    helper.write_text(f"#!{sys.executable}\nimport sys\nsys.stdin.readline()\nprint('PAM_PROMPT_ECHO_OFF Password:', flush=True)\nsys.stdin.readline()\nprint('FAILURE', flush=True)\n")
    helper.chmod(0o755)
    os.environ["PATH"] = f"{bindir}:/apps/bin:" + os.environ["PATH"]
