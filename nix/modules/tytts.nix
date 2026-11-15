{self}:
import ./lib.nix {inherit self;} {
  name = "tytts";
  description = "tytts local speech with Pocket TTS, Zonos3, Kokoro, Piper, and Qwen3-TTS";
  hasService = false;
  settingsExample = {
    engine = "pocket";
    auto_download = true;
    speed = 1.0;
  };
}
