{self}:
import ./lib.nix {inherit self;} {
  name = "tyci";
  description = "tyci GitHub PR and CI tray applet";
  settingsExample = {
    poll_seconds = 60;
    repos = {
      include = ["tylergets" "my-org/api-*"];
      exclude = ["tylergets/old-thing"];
    };
    pull_requests.query = "involves:@me";
  };
}
