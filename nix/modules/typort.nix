{self}:
import ./lib.nix {inherit self;} {
  name = "typort";
  description = "typort listening ports tray applet";
  settingsExample = {
    ignore_ports = [631 8384];
    ignore_processes = ["kdeconnectd"];
    https_ports = [8443];
    browser = ["firefox" "--new-tab"];
  };
}
