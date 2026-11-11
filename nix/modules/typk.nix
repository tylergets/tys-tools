{self}:
import ./lib.nix {inherit self;} {
  name = "typk";
  description = "typk polkit authentication agent";
  settingsExample = {
    max_attempts = 3;
    show_details = false;
    dim_screen = false;
  };
}
