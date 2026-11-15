{self}:
import ./lib.nix {inherit self;} {
  name = "tybat";
  description = "Battery monitor with focus-grabbing critical countdowns";
  settingsExample = {
    warning_percent = 20;
    critical_percent = 5;
    countdown_seconds = 60;
    snooze_seconds = 60;
    poll_seconds = 5;
    action = "suspend";
  };
}
