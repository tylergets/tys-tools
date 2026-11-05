{self}:
import ./lib.nix {inherit self;} {
  name = "tycapture";
  description = "tycapture screen recording applet";
  settingsExample = {
    recording = {
      output_dir = "~/Videos/tycapture";
      format = "mp4";
      framerate = 60;
    };
    audio.microphone = true;
    window.workspace = "rec";
  };
}
