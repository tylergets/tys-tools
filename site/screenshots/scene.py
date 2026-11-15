"""One fresh X server and bus per scene; never runs on the user's desktop."""
import json
import os
from pathlib import Path
import subprocess
import signal
import sys
import time

os.environ["DBUS_SYSTEM_BUS_ADDRESS"] = os.environ["DBUS_SESSION_BUS_ADDRESS"]
Path("/run/user").mkdir(mode=0o700)
xserver = subprocess.Popen(["Xvfb", ":99", "-screen", "0", "1200x800x24", "-dpi", "96", "-nolisten", "tcp", "-ac"])
for _ in range(100):
    if Path("/tmp/.X11-unix/X99").exists():
        break
    time.sleep(.05)
else:
    raise SystemExit("Xvfb did not start")

import gi
gi.require_version("Gtk", "3.0")
gi.require_version("DbusmenuGtk3", "0.4")
from gi.repository import Gtk, Gdk, GLib, DbusmenuGtk3
from PIL import Image, ImageChops
import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from fixtures import prepare

DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
scene = sys.argv[1]
tool = json.loads(Path('/capture/scenes.json').read_text())[scene]['tool']
prepare(tool)
subprocess.Popen(["openbox", "--sm-disable"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def pump(seconds=.1):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        time.sleep(.01)


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=15)
    if result.returncode:
        raise RuntimeError(f"{args}: {result.stderr}")
    return result.stdout


def launch(*args):
    return subprocess.Popen(args, stdout=sys.stdout, stderr=sys.stderr)


def wait_for(predicate, description, seconds=15):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if predicate():
            return
        pump()
    raise RuntimeError(f"Timed out waiting for {description}")


class Watcher(dbus.service.Object):
    def __init__(self):
        self.name = dbus.service.BusName("org.kde.StatusNotifierWatcher", bus)
        super().__init__(bus, "/StatusNotifierWatcher")
        self.items = []

    @dbus.service.method("org.kde.StatusNotifierWatcher", in_signature="s", sender_keyword="sender")
    def RegisterStatusNotifierItem(self, service, sender=None):
        name, path = (sender, service) if service.startswith("/") else (service, "/StatusNotifierItem")
        self.items.append((str(name), str(path)))
        self.StatusNotifierItemRegistered(name + path)

    @dbus.service.method("org.kde.StatusNotifierWatcher", in_signature="s")
    def RegisterStatusNotifierHost(self, name):
        pass

    @dbus.service.signal("org.kde.StatusNotifierWatcher", signature="s")
    def StatusNotifierItemRegistered(self, name):
        pass

    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v")
    def Get(self, interface, prop):
        return self.GetAll(interface)[prop]

    @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface):
        return {"IsStatusNotifierHostRegistered": dbus.Boolean(True), "ProtocolVersion": dbus.Int32(0),
                "RegisteredStatusNotifierItems": dbus.Array([name + path for name, path in self.items], signature="s")}


watcher = Watcher()
window = Gtk.Window(title="ty's tools · sample desktop")
window.set_decorated(False)
window.set_default_size(1200, 800)
window.move(0, 0)
canvas = Gtk.DrawingArea()
window.add(canvas)


def draw(_, cr):
    cr.set_source_rgb(.075, .085, .105)
    cr.paint()
    return False


canvas.connect("draw", draw)
window.show_all()
pump(.5)
def snapshot():
    pixbuf = Gdk.pixbuf_get_from_window(Gdk.get_default_root_window(), 0, 0, 1200, 800)
    mode = "RGBA" if pixbuf.get_has_alpha() else "RGB"
    return Image.frombytes(mode, (1200, 800), bytes(pixbuf.get_pixels()), "raw", mode, pixbuf.get_rowstride()).convert("RGB")


baseline = snapshot()


def tray_menu():
    wait_for(lambda: watcher.items, "tray registration")
    pump(2)
    name, path = watcher.items[-1]
    obj = bus.get_object(name, path)
    menu_path = obj.Get("org.kde.StatusNotifierItem", "Menu", dbus_interface="org.freedesktop.DBus.Properties")
    menu = DbusmenuGtk3.Menu.new(name, str(menu_path))
    menu.show_all()
    pump(1)
    rect = Gdk.Rectangle()
    size = menu.get_preferred_size()[1]
    # Leave room for the real submenu, keeping the combined scene in-frame.
    left = {"tyboard": 350, "tyspeak": 200, "typort": 320, "tylight": 350}.get(tool, (1200 - size.width) // 2)
    rect.x, rect.y, rect.width, rect.height = left, max(80, (800 - size.height) // 2), 1, 1
    menu.popup_at_rect(window.get_window(), rect, Gdk.Gravity.NORTH_WEST, Gdk.Gravity.NORTH_WEST, None)
    pump(1)
    return menu


def clipboard(text):
    cb = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
    cb.set_text(text, -1)
    pump(.6)


def open_submenu(menu, prefix):
    for child in menu.get_children():
        if (child.get_label() or "").startswith(prefix):
            _, x, y = menu.get_window().get_origin()
            bounds = child.get_allocation()
            run("xdotool", "mousemove", str(x + bounds.x + 30), str(y + bounds.y + bounds.height // 2))
            pump(1)
            if not child.get_submenu().get_mapped():
                raise RuntimeError(f"Submenu did not open: {prefix}")
            return
    raise RuntimeError(f"Missing submenu: {prefix}")


if scene == "tybat-tray":
    class Device(dbus.service.Object):
        def __init__(self, index, model, percent, laptop):
            self.path = f"/org/freedesktop/UPower/devices/sample{index}"
            super().__init__(bus, self.path)
            self.props = {"Type": dbus.UInt32(2 if laptop else 5), "IsPresent": dbus.Boolean(True),
                          "PowerSupply": dbus.Boolean(laptop), "Model": model,
                          "Percentage": dbus.Double(percent), "State": dbus.UInt32(2), "NativePath": self.path}

        @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="s", out_signature="a{sv}")
        def GetAll(self, interface):
            return self.props

    class UPower(dbus.service.Object):
        def __init__(self):
            self.name = dbus.service.BusName("org.freedesktop.UPower", bus)
            super().__init__(bus, "/org/freedesktop/UPower")
            self.devices = [Device(0, "Framework", 76, True), Device(1, "Wireless mouse", 62, False), Device(2, "Headphones", 85, False)]

        @dbus.service.method("org.freedesktop.UPower", out_signature="ao")
        def EnumerateDevices(self):
            return [dbus.ObjectPath(device.path) for device in self.devices]

    power = UPower()
    config = Path.home() / '.config/tybat/config.yaml'
    config.write_text(config.read_text() + "charge_limit: { percent: 80, command: ['true'] }\n")
    app = launch(tool, "run")
    menu = tray_menu()
    open_submenu(menu, "Maximum charge:")
elif scene == "tylight-graph":
    app = launch(tool, "run")
    wait_for(lambda: watcher.items, "applet")
    run("tylight", "graph")
    pump(2)
elif tool in {"tyboard", "tykey", "tyspeak", "tylight", "tyci", "typort"}:
    if tool == "typort":
        Path("index.html").write_text("<title>Project documentation</title><h1>Documentation</h1>")
        subprocess.Popen(["python", "-m", "http.server", "5173", "--bind", "127.0.0.1"], executable=sys.executable)
    app = launch(tool, "run")
    wait_for(lambda: watcher.items, "applet")
    if tool == "tyboard":
        for text in ("https://example.com/project", "Make room for the work that matters.", "git switch -c a-small-improvement"):
            clipboard(text)
    if tool == "tylight":
        pump(1)
        run("tylight", "set", "4500")
    pump(3)
    menu = tray_menu()
    if tool == "tyboard":
        open_submenu(menu, "History")
    elif tool == "tyspeak":
        open_submenu(menu, "Model:")
    elif tool == "typort":
        open_submenu(menu, ":5173")
    elif tool == "tylight":
        open_submenu(menu, "Presets")
elif tool == "tybat":
    app = launch("tybat", "preview")
    pump(.5)
    os.kill(app.pid, signal.SIGSTOP)
elif tool == "tycapture":
    app = launch("tycapture", "run")
    wait_for(lambda: Path("/run/user/tycapture-demo.sock").exists(), "capture daemon")
    if scene == "tycapture-recording":
        run("tycapture", "start", "screen", "--now")
        wait_for(lambda: run("tycapture", "status").startswith("recording"), "recording")
        menu = tray_menu()
    else:
        launch("tycapture", "start", "screen")
        pump(2)
elif tool == "tyshot":
    # Use a real GTK window as the subject, then select it with the actual overlay.
    note = Gtk.Window(title="A small improvement")
    note.set_default_size(640, 380)
    note.move(280, 190)
    text = Gtk.Label(label="A small improvement\n\nKeep the useful parts.\nMake the next step obvious.\nLeave things a little better.")
    text.set_margin_top(48)
    text.set_margin_bottom(48)
    text.set_margin_start(48)
    text.set_margin_end(48)
    note.add(text)
    note.show_all()
    pump(.5)
    app = launch("tyshot", "gui")
    pump(1)
    run("xdotool", "mousemove", "250", "160", "mousedown", "1", "mousemove", "950", "620", "mouseup", "1")
    pump(.5)
    if scene == "tyshot-annotate":
        for key, start, end in [("r", (420, 325), (785, 465)), ("a", (810, 505), (715, 425))]:
            run("xdotool", "key", key)
            run("xdotool", "mousemove", *map(str, start), "mousedown", "1", "mousemove", *map(str, end), "mouseup", "1")
            pump(.2)
        run("xdotool", "key", "n", "mousemove", "395", "340", "click", "1")
        pump(.3)
elif tool == "tygesture":
    app = launch("tygesture", "run")
    wait_for(lambda: Path("/run/user/tygesture-demo.sock").exists(), "gesture daemon")
    run("tygesture", "show")
    pump(1)
    wid = run("xdotool", "search", "--onlyvisible", "--name", "^tygesture$").strip().splitlines()[-1]
    run("xdotool", "windowmove", wid, "210", "185")
elif tool == "tynotif":
    app = launch("tynotif", "run")
    wait_for(lambda: bus.name_has_owner("org.freedesktop.Notifications"), "notifications")
    pump(.5)
    run("tynotif", "close-all")
    proxy = bus.get_object("org.freedesktop.Notifications", "/org/freedesktop/Notifications")
    notify = dbus.Interface(proxy, "org.freedesktop.Notifications")
    notify.Notify("Build & test", 0, "", "All checks passed", "Your changes are ready to ship.", ["default", "Open build"], {"urgency": dbus.Byte(1)}, 0)
    notify.Notify("Downloads", 0, "", "Download in progress", "Release archive", [], {"value": dbus.Int32(68)}, 0)
    pump(1)
    if scene == "tynotif-history":
        run("tynotif", "close-all")
        menu = tray_menu()
        open_submenu(menu, "History (")
elif tool == "tylock":
    app = launch("tylock")
    pump(1)
elif tool == "tyquote":
    run("tyquote", "add", "Make room for the work that matters.", "--author", "A reminder")
    script = Path("quote-session.sh")
    script.write_text("set -e\nprintf '\\033[1;36m$ tyquote latest\\033[0m\\n\\n'\n/apps/bin/tyquote latest\nprintf '\\n\\033[1;36m$ tyquote today\\033[0m\\n\\n'\n/apps/bin/tyquote today\ntouch /home/demo/quote-ready\nread -r _\n")
    app = launch("xterm", "-geometry", "84x20+180+170", "-fa", "DejaVu Sans Mono", "-fs", "13", "-bg", "#171b22", "-fg", "#d3d2d1", "-title", "tyquote", "-e", "/bin/sh", str(script))
    wait_for(lambda: Path("quote-ready").exists(), "quote terminal output")
    pump(1)
elif tool == "typk":
    # A private fake authority sends a normal request to the real agent. The helper
    # only prompts and always fails; no host polkit/PAM services are reachable.
    class Authority(dbus.service.Object):
        def __init__(self):
            self.name = dbus.service.BusName("org.freedesktop.PolicyKit1", bus)
            super().__init__(bus, "/org/freedesktop/PolicyKit1/Authority")
            self.agent = None

        @dbus.service.method("org.freedesktop.PolicyKit1.Authority", in_signature="(sa{sv})so", sender_keyword="sender")
        def RegisterAuthenticationAgent(self, subject, locale, path, sender=None):
            self.agent = (sender, path)

        @dbus.service.method("org.freedesktop.PolicyKit1.Authority", in_signature="(sa{sv})o")
        def UnregisterAuthenticationAgent(self, subject, path):
            pass

    authority = Authority()
    app = launch("typk", "run")
    wait_for(lambda: authority.agent, "polkit agent registration")
    name, path = authority.agent
    proxy = bus.get_object(name, path)
    identities = dbus.Array([dbus.Struct(("unix-user", dbus.Dictionary({"uid": dbus.UInt32(os.getuid())}, signature="sv")), signature="sa{sv}")], signature="(sa{sv})")
    proxy.BeginAuthentication("tools.tys.typk.verify", "Authentication is required to install updates.", "system-software-update", dbus.Dictionary({}, signature="ss"), "sample-cookie", identities, dbus_interface="org.freedesktop.PolicyKit1.AuthenticationAgent", reply_handler=lambda: None, error_handler=lambda error: None)
    pump(1)
else:
    raise RuntimeError(f"No scene for {tool}")

if app.poll() is not None:
    raise RuntimeError(f"{tool} exited before capture ({app.returncode})")
run("xdotool", "mousemove", "1180", "780")
pump(.2)
image = snapshot()
bounds = ImageChops.difference(image, baseline).getbbox()
if bounds is None:
    raise RuntimeError(f"{tool} left only the empty sample desktop visible")
if max(high - low for low, high in image.getextrema()) < 8:
    raise RuntimeError(f"{tool} rendered a blank screen")
# Menus and dialogs need a close-up, not a made-up desktop frame. Keep native
# window decorations where the app actually has them. Screen-wide UIs stay wide.
framing = "full-screen"
if tool not in {"tylock", "tyshot"}:
    if scene == "tybat":
        bounds = (320, 245, 880, 555)  # critical popup; exclude the dimming surface
    else:
        # Sample the final backdrop: initial WM/theme settling can repaint the
        # entire desktop after the baseline, which would defeat a temporal diff.
        backdrop = Image.new("RGB", image.size, image.getpixel((0, 799)))
        bounds = ImageChops.difference(image, backdrop).getbbox()
        if bounds is None:
            raise RuntimeError(f"{tool} has no visible content to crop")
    left, top, right, bottom = bounds
    image = image.crop((max(0, left - 12), max(0, top - 12), min(1200, right + 12), min(800, bottom + 12)))
    framing = "content"
image.save(f"/output/{scene}.png")
version = run(tool, "--version").strip()
Path(f"/output/{scene}.json").write_text(json.dumps({"version": version, "width": image.width, "height": image.height, "framing": framing, "scene": scene, "theme": "Adwaita:dark", "font": "DejaVu Sans 11"}, indent=2) + "\n")
print(f"Captured {tool}: {version}", flush=True)
os._exit(0)
