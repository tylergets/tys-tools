{self}:
import ./lib.nix {inherit self;} {
  name = "tylight";
  description = "tylight screen color temperature daemon";
  settingsExample = {
    temperature = {
      day = 6500;
      night = 3500;
    };
    location = {
      latitude = 39.1;
      longitude = -84.5;
    };
    fade_seconds = 2;
  };
}
