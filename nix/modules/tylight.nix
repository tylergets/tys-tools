{self}:
import ./lib.nix {inherit self;} {
  name = "tylight";
  description = "tylight screen color temperature daemon";
  settingsExample = {
    schedule = [
      {
        name = "day";
        elevation = 6;
        temperature = 6500;
      }
      {
        name = "evening";
        elevation = -4;
        temperature = 4500;
      }
      {
        name = "night";
        elevation = -12;
        temperature = 3200;
        brightness = 0.9;
      }
    ];
    location = {
      latitude = 39.1;
      longitude = -84.5;
    };
    fade_seconds = 2;
    app_whitelist = ["steam"];
  };
}
