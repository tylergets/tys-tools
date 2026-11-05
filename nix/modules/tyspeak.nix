{self}:
import ./lib.nix {inherit self;} {
  name = "tyspeak";
  description = "tyspeak speech-to-text dictation daemon";
  settingsExample = {
    model = "parakeet-unified-en";
    audio.source = "default";
    output = {
      mode = "type";
      trailing_space = true;
    };
  };
}
