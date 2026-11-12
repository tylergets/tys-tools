{self}:
import ./lib.nix {inherit self;} {
  name = "tynotif";
  description = "tynotif desktop notification daemon";
  settingsExample = {
    width = 480;
    limit = 0; # Fill the available screen height.
    timeout_ms = 20000;
    history_limit = 200;
  };
}
