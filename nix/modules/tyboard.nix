{self}:
import ./lib.nix {inherit self;} {
  name = "tyboard";
  description = "tyboard clipboard history daemon";
  settingsExample = {
    clipboard.active_window.blacklist = ["KeePassXC"];
  };
}
