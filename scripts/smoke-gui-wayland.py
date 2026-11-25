#!/usr/bin/env python3
"""Native Wayland file-drop smoke in an isolated headless Sway. No Factory services.

Requires sway, swaymsg, grim, wtype, a C compiler, wayland-scanner, libwayland-client
and /usr/bin/python3 with GTK3 PyGObject.
Never connects to the user's compositor or modifies the user's settings.
"""
import argparse
import base64
import json
import os
from pathlib import Path
from cargo_paths import profile_dir
import shutil
import signal
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]

# Minimal client-side wire description of wlr-virtual-pointer-v1. Only used on
# our private test compositor, never on the user's desktop. No input privileges.
POINTER_PROTOCOL = '''<protocol name="test_virtual_pointer">
<interface name="zwlr_virtual_pointer_v1" version="1">
 <request name="motion"><arg name="time" type="uint"/><arg name="dx" type="fixed"/><arg name="dy" type="fixed"/></request>
 <request name="motion_absolute"><arg name="time" type="uint"/><arg name="x" type="uint"/><arg name="y" type="uint"/><arg name="width" type="uint"/><arg name="height" type="uint"/></request>
 <request name="button"><arg name="time" type="uint"/><arg name="button" type="uint"/><arg name="state" type="uint"/></request>
 <request name="axis"><arg name="time" type="uint"/><arg name="axis" type="uint"/><arg name="value" type="fixed"/></request>
 <request name="frame"/>
 <request name="axis_source"><arg name="source" type="uint"/></request>
 <request name="axis_stop"><arg name="time" type="uint"/><arg name="axis" type="uint"/></request>
 <request name="axis_discrete"><arg name="time" type="uint"/><arg name="axis" type="uint"/><arg name="value" type="fixed"/><arg name="discrete" type="int"/></request>
 <request name="destroy" type="destructor"/>
</interface>
<interface name="zwlr_virtual_pointer_manager_v1" version="1">
 <request name="create_virtual_pointer"><arg name="seat" type="object" interface="wl_seat" allow-null="true"/><arg name="pointer" type="new_id" interface="zwlr_virtual_pointer_v1"/></request>
 <request name="destroy" type="destructor"/>
</interface></protocol>'''
POINTER_CLIENT = r'''
#include <wayland-client.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include "pointer.h"
static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *d,struct wl_registry *r,uint32_t name,const char *iface,uint32_t version) {
 if (!strcmp(iface,"zwlr_virtual_pointer_manager_v1")) manager=wl_registry_bind(r,name,&zwlr_virtual_pointer_manager_v1_interface,1);
}
static void removed(void *d,struct wl_registry *r,uint32_t name) {}
int main(void) {
 struct wl_display *display=wl_display_connect(NULL);
 if(!display) return 1;
 struct wl_registry *registry=wl_display_get_registry(display);
 const struct wl_registry_listener listener={global,removed};
 wl_registry_add_listener(registry,&listener,NULL);wl_display_roundtrip(display);
 if(!manager) return 2;
 struct zwlr_virtual_pointer_v1 *pointer=zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager,NULL);
 wl_display_roundtrip(display);puts("ready");fflush(stdout);
 char op[32];unsigned a,b;
 while(scanf("%31s %u %u",op,&a,&b)==3) {
  struct timespec now;clock_gettime(CLOCK_MONOTONIC,&now);
  uint32_t ms=now.tv_sec*1000+now.tv_nsec/1000000;
  if(!strcmp(op,"move")) zwlr_virtual_pointer_v1_motion_absolute(pointer,ms,a,b,1600,1000);
  else zwlr_virtual_pointer_v1_button(pointer,ms,272,a);
  zwlr_virtual_pointer_v1_frame(pointer);wl_display_roundtrip(display);puts("ok");fflush(stdout);
 }
 zwlr_virtual_pointer_v1_destroy(pointer);wl_display_disconnect(display);return 0;
}'''


def wait_for(predicate, description, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.05)
    raise AssertionError(f"Timed out: {description}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--uri-only", action="store_true", help="Test the URI-only baseline instead of file-manager multi-MIME offers")
    parser.add_argument("--thunar", action="store_true", help="Use a real isolated Thunar instance instead of the GTK fixture")
    parser.add_argument("--covered-clipboard",action="store_true",help="Read copied text with the GUI completely covered by the receiving application")
    args = parser.parse_args()
    variant = "covered-clipboard" if args.covered_clipboard else "thunar" if args.thunar else "uri-only" if args.uri_only else "multi-mime"
    for executable in ("sway", "swaymsg", "grim", "wtype", "wayland-scanner", "cc", "pkg-config"):
        if not shutil.which(executable):
            raise SystemExit(f"Missing test dependency: {executable}")
    if args.thunar and (not shutil.which("thunar") or not shutil.which("dbus-run-session")):
        raise SystemExit("Missing test dependency: thunar / dbus-run-session")
    output = Path(os.environ.get("YOLOFORGE_WAYLAND_SMOKE_OUTPUT", "/tmp/opencode" if Path("/tmp/opencode").exists() else tempfile.gettempdir()))
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="yoloforge-wayland-", dir="/tmp/opencode" if Path("/tmp/opencode").exists() else None) as temporary:
        directory = Path(temporary)
        (directory / "pointer.xml").write_text(POINTER_PROTOCOL)
        (directory / "pointer.c").write_text(POINTER_CLIENT)
        for kind, generated in (("client-header", "pointer.h"), ("private-code", "protocol.c")):
            subprocess.run(["wayland-scanner", kind, str(directory / "pointer.xml"), str(directory / generated)], check=True)
        flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "wayland-client"], text=True).split()
        subprocess.run(["cc", str(directory / "pointer.c"), str(directory / "protocol.c"), "-o", str(directory / "pointer"), *flags], check=True)
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        data = directory / "data" / "yoloforge"
        data.mkdir(parents=True)
        settings = data / "desktop.json"
        settings.write_text(json.dumps({"server": "http://127.0.0.1:1", "project": str(uuid.uuid4())}))
        source_folder = directory / "drop-source"
        source_folder.mkdir()
        note = source_folder / "Notizen Grüße.txt"
        note.write_text("Lokaler Drag-and-drop-Test — kein Server")
        picture = source_folder / "Bild.png"
        picture.write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="))
        config = directory / "sway.conf"
        config.write_text('output * mode 1600x1000\nseat seat0 fallback true\n'
                          'for_window [title="^YoloForge$"] floating enable, resize set 1100 850, move position 400 50\n'
                          'for_window [title="^Drop Fixture$"] floating enable, resize set 280 300, move position 20 50\n')
        with config.open("a") as config_file:
            config_file.write('for_window [app_id="(?i)thunar|org.xfce.Thunar"] floating enable, resize set 360 600, move position 20 50\n')
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), WLR_BACKENDS="headless", WLR_RENDERER="pixman",
                   WLR_LIBINPUT_NO_DEVICES="1", XDG_DATA_HOME=str(directory / "data"),
                   XDG_CONFIG_HOME=str(directory / "config"), FACTORY_TOKEN="isolated-offline-fixture-no-server-token", GDK_BACKEND="wayland")
        env["YOLOFORGE_DND_URI_ONLY"] = "1" if args.uri_only else "0"
        env["YOLOFORGE_TEST_CLIPBOARD"] = str(directory / "clipboard.txt")
        env["YOLOFORGE_TEST_COVERED"] = "1" if args.covered_clipboard else "0"
        for key in ("WAYLAND_DISPLAY", "DISPLAY", "SWAYSOCK", "FACTORY_TOKEN_FILE"):
            env.pop(key, None)
        processes = []
        with (directory / "compositor.log").open("w+") as compositor_log, (directory / "gui.log").open("w+") as gui_log:
            try:
                processes.append(subprocess.Popen(["sway", "-c", str(config)], env=env, stdout=compositor_log, stderr=compositor_log))
                socket = wait_for(lambda: next(runtime.glob("sway-ipc.*.sock"), None), "isolated compositor")
                wayland = wait_for(lambda: next((p for p in runtime.glob("wayland-*") if not p.name.endswith(".lock")), None), "Wayland socket")
                env.update(SWAYSOCK=str(socket), WAYLAND_DISPLAY=wayland.name)
                pointer_process = subprocess.Popen([str(directory / "pointer")], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
                processes.append(pointer_process)
                assert pointer_process.stdout.readline().strip() == "ready"

                def pointer(operation, a, b=0):
                    pointer_process.stdin.write(f"{operation} {a} {b}\n")
                    pointer_process.stdin.flush()
                    assert pointer_process.stdout.readline().strip() == "ok"

                def ipc(command):
                    result = json.loads(subprocess.check_output(["swaymsg", "-r", command], env=env))
                    assert all(item.get("success") for item in result), result
                    return result

                # Use explicit argv for IPC queries (not a shell command).
                def window(title):
                    tree = json.loads(subprocess.check_output(["swaymsg", "-r", "-t", "get_tree"], env=env))
                    def find(node):
                        if node.get("name") == title or (title == "THUNAR" and "thunar" in (node.get("app_id") or "").lower()):
                            return node
                        for child in node.get("nodes", []) + node.get("floating_nodes", []):
                            result = find(child)
                            if result:
                                return result
                    return find(tree)

                gui = subprocess.Popen([str(profile_dir() / "yoloforge-gui"), "--server", "http://127.0.0.1:1"], env=env, stdout=gui_log, stderr=gui_log)
                processes.append(gui)
                target = wait_for(lambda: window("YoloForge"), "SDL3 Wayland window")
                assert target.get("shell") == "xdg_shell", target
                rect = target["rect"]
                time.sleep(.5)
                # Open a local task draft using the actual GUI button.
                pointer("move", rect["x"] + 100, rect["y"] + 255)
                pointer("button", 1)
                time.sleep(.1)
                pointer("button", 0)
                time.sleep(.3)
                subprocess.run(["wtype", "-s", "300", "Grüße €", "-s", "200", "-M", "ctrl", "-k", "a", "-s", "100", "-k", "c", "-m", "ctrl",
                                "-s", "200", "-k", "Right", "-M", "ctrl", "-k", "v", "-m", "ctrl"], env=env, check=True)
                time.sleep(.2)
                source = directory / "source.py"
                source.write_text('''import gi,sys,os
gi.require_version("Gtk","3.0")
from gi.repository import Gtk,Gdk,GLib
from pathlib import Path
w=Gtk.Window(title="Drop Fixture")
area=Gtk.EventBox(); area.add(Gtk.Label(label="Drag fixture files")); w.add(area)
targets=[Gtk.TargetEntry.new("text/uri-list",0,0)]
if os.environ["YOLOFORGE_DND_URI_ONLY"] != "1": targets += [Gtk.TargetEntry.new("text/plain;charset=utf-8",0,1),Gtk.TargetEntry.new("text/plain",0,1)]
area.drag_source_set(Gdk.ModifierType.BUTTON1_MASK,targets,Gdk.DragAction.COPY)
def transfer(widget,context,selection,info,timestamp):
 if info == 0: selection.set_uris(sys.argv[1:])
 else:
  from urllib.parse import urlparse,unquote
  selection.set_text("\\n".join(unquote(urlparse(uri).path) for uri in sys.argv[1:]),-1)
area.connect("drag-data-get",transfer)
w.connect("destroy",Gtk.main_quit); w.show_all()
if os.environ["YOLOFORGE_TEST_COVERED"] == "1": w.fullscreen()
def received(clipboard,text,data):
 Path(os.environ["YOLOFORGE_TEST_CLIPBOARD"]).write_text(text or "")
 w.unfullscreen()
def read_clipboard():
 Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).request_text(received,None)
 return False
GLib.timeout_add(1500 if os.environ["YOLOFORGE_TEST_COVERED"] == "1" else 300,read_clipboard)
Gtk.main()
''')
                if args.thunar:
                    thunar_env = dict(env, HOME=str(directory / "home"), XDG_CACHE_HOME=str(directory / "cache"), XDG_STATE_HOME=str(directory / "state"), GIO_USE_VFS="local", GIO_USE_VOLUME_MONITOR="unix", NO_AT_BRIDGE="1")
                    processes.append(subprocess.Popen(["dbus-run-session", "--", "thunar", str(source_folder)], env=thunar_env, start_new_session=True, stdout=compositor_log, stderr=compositor_log))
                    src = wait_for(lambda: window("THUNAR"), "isolated Thunar")["rect"]
                    time.sleep(.5)
                    sx, sy = src["x"] + 235, src["y"] + 135
                else:
                    processes.append(subprocess.Popen(["/usr/bin/python3", str(source), picture.as_uri(), note.as_uri()], env=env))
                    src = wait_for(lambda: window("Drop Fixture"), "GTK Wayland drag source")["rect"]
                    wait_for(lambda:(directory / "clipboard.txt").exists(),"clipboard delivery to a different native Wayland application")
                    assert (directory / "clipboard.txt").read_text() == "Grüße €", "Copied text must be available outside the GUI"
                    if args.covered_clipboard:
                        wait_for(lambda:window("Drop Fixture")["rect"]["width"]<400,"receiver leaves fullscreen")
                        ipc('[title="^YoloForge$"] kill')
                        assert gui.wait(timeout=10)==0
                        saved=json.loads(settings.read_text())
                        assert saved["task_drafts"][0]["prompt"]=="Grüße €Grüße €"
                        assert not saved["pending"]
                        print("Native Wayland: copied text delivered to a different application while the GUI was fully covered; draft preserved.")
                        return
                    sx, sy = src["x"] + 100, src["y"] + 120
                tx, ty = rect["x"] + 550, rect["y"] + 400
                # Thunar gets two individual file drags; the GTK fixture covers
                # one multi-file offer without depending on file-manager selection.
                starts = [(sx, sy), (sx, src["y"] + 230)] if args.thunar else [(sx, sy)]
                for sx, sy in starts:
                    pointer("move", sx, sy)
                    pointer("button", 1)
                    time.sleep(.1)
                    for x, y in [(sx+30, sy+20), (sx+60, sy+40), (tx, ty)]:
                        pointer("move", x, y)
                        time.sleep(.15)
                    subprocess.run(["grim", str(directory / "hover.png")], env=env, check=True)
                    pointer("button", 0)
                    time.sleep(1)
                subprocess.run(["grim", str(directory / "attached.png")], env=env, check=True)
                ipc('[title="^YoloForge$"] kill')
                assert gui.wait(timeout=10) == 0
                saved = json.loads(settings.read_text())
                assert len(saved["task_drafts"]) == 1, saved
                draft = saved["task_drafts"][0]
                assert draft["prompt"] == "Grüße €Grüße €", draft["prompt"]
                assert [i["name"] for i in draft["images"]] == [picture.name], draft
                assert [f["name"] for f in draft["files"]] == [note.name], draft
                assert not saved["pending"], "Drop must not submit anything"
                gui_log.seek(0)
                assert "SDL3-Fensterbackend wayland" in gui_log.read()
                for name in ("hover", "attached"):
                    shutil.copy2(directory / f"{name}.png", output / f"yoloforge-wayland-{variant}-{name}.png")
                print(f"Native Wayland ({variant}): hover, file drop, Unicode input, clipboard, image/file draft and close persistence passed; no Factory services started.")
            except Exception:
                for name in ("hover", "attached"):
                    if (directory / f"{name}.png").exists():
                        shutil.copy2(directory / f"{name}.png", output / f"yoloforge-wayland-{variant}-{name}.png")
                for log in (compositor_log, gui_log):
                    log.seek(0)
                    print(log.read())
                raise
            finally:
                for process in reversed(processes):
                    if process.poll() is None:
                        if args.thunar and process.args[0] == "dbus-run-session":
                            os.killpg(process.pid, signal.SIGTERM)
                        process.terminate()
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()


if __name__ == "__main__":
    main()
