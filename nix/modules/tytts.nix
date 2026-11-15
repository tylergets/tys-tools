{self}:
import ./lib.nix {inherit self;} {
  name = "tytts";
  description = "tytts local text-to-speech";
  hasService = false;
  settingsExample = {
    auto_download = true;
    speed = 1.0;
  };
}
