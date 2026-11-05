{self}:
import ./lib.nix {inherit self;} {
  name = "tyshot";
  description = "tyshot screenshot tool";
  serviceDescription = "tyshot screenshot tray applet";
  serviceArgs = "tray";
  settingsExample = {
    save.output_dir = "~/Pictures/Screenshots";
    overlay.color = "#3e63dd";
    toolbar.button_size = 56;
    notifications.enabled = false;
  };
}
