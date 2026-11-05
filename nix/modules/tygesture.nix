# tygesture has no typed options; bindings go in `settings` (see the tygesture README).
{self}:
import ./lib.nix {inherit self;} {
  name = "tygesture";
  description = "tygesture touchpad gesture daemon";
  settingsExample = {
    swipe."3" = {
      left = "xdotool key alt+Right";
      right = "xdotool key alt+Left";
    };
    swipe."4".up = {
      command = "pactl set-sink-volume @DEFAULT_SINK@ +5%";
      repeat = true;
    };
    pinch."2" = {
      "in" = "xdotool key ctrl+minus";
      out = "xdotool key ctrl+plus";
    };
  };
}
