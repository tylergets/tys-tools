//! tygesture config: thresholds plus command bindings per gesture and finger count.

use std::collections::BTreeMap;
use std::fmt;

use serde::{de, Deserialize, Deserializer};

use crate::gesture::{Dir, Gesture, Spin, Zoom};

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Show the tray icon while running.
    pub tray: bool,
    pub thresholds: Thresholds,
    pub swipe: BTreeMap<Fingers, Swipe>,
    pub pinch: BTreeMap<Fingers, Pinch>,
    pub rotate: BTreeMap<Fingers, Rotate>,
    pub hold: BTreeMap<Fingers, Binding>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tray: true,
            thresholds: Thresholds::default(),
            swipe: BTreeMap::new(),
            pinch: BTreeMap::new(),
            rotate: BTreeMap::new(),
            hold: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    /// Finger travel for a swipe to count, in libinput's normalized units (1000 per inch).
    pub swipe: f64,
    /// Travel between firings of a `repeat` swipe binding.
    pub swipe_step: f64,
    /// Relative scale change for a pinch: 0.2 means "in" below 0.8 and "out" above 1.2.
    pub pinch: f64,
    /// Degrees of rotation for a rotate (also the step for `repeat` rotate bindings).
    pub rotate: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            swipe: 200.0,
            swipe_step: 120.0,
            pinch: 0.2,
            rotate: 25.0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Swipe {
    pub left: Option<Binding>,
    pub right: Option<Binding>,
    pub up: Option<Binding>,
    pub down: Option<Binding>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pinch {
    #[serde(rename = "in")]
    pub pinch_in: Option<Binding>,
    #[serde(rename = "out")]
    pub pinch_out: Option<Binding>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rotate {
    pub cw: Option<Binding>,
    pub ccw: Option<Binding>,
}

/// A shell command, optionally repeating while the gesture continues.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub command: String,
    /// Fire every step of movement during the gesture instead of once at the end.
    pub repeat: bool,
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            command: String,
            #[serde(default)]
            repeat: bool,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Command(String),
            Full(Full),
        }

        Ok(match Raw::deserialize(deserializer)? {
            Raw::Command(command) => Binding {
                command,
                repeat: false,
            },
            Raw::Full(full) => Binding {
                command: full.command,
                repeat: full.repeat,
            },
        })
    }
}

/// A finger count map key. Accepts `3` and `"3"`, since Nix's YAML generator quotes attr names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fingers(pub u32);

impl<'de> Deserialize<'de> for Fingers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl de::Visitor<'_> for Visitor {
            type Value = Fingers;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a finger count")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Fingers, E> {
                u32::try_from(v).map(Fingers).map_err(E::custom)
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Fingers, E> {
                u32::try_from(v).map(Fingers).map_err(E::custom)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Fingers, E> {
                v.trim()
                    .parse()
                    .map(Fingers)
                    .map_err(|_| E::custom(format!("invalid finger count: {v:?}")))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

impl Config {
    pub fn binding(&self, gesture: Gesture) -> Option<&Binding> {
        match gesture {
            Gesture::Swipe(fingers, dir) => {
                let swipe = self.swipe.get(&Fingers(fingers))?;
                match dir {
                    Dir::Left => swipe.left.as_ref(),
                    Dir::Right => swipe.right.as_ref(),
                    Dir::Up => swipe.up.as_ref(),
                    Dir::Down => swipe.down.as_ref(),
                }
            }
            Gesture::Pinch(fingers, zoom) => {
                let pinch = self.pinch.get(&Fingers(fingers))?;
                match zoom {
                    Zoom::In => pinch.pinch_in.as_ref(),
                    Zoom::Out => pinch.pinch_out.as_ref(),
                }
            }
            Gesture::Rotate(fingers, spin) => {
                let rotate = self.rotate.get(&Fingers(fingers))?;
                match spin {
                    Spin::Cw => rotate.cw.as_ref(),
                    Spin::Ccw => rotate.ccw.as_ref(),
                }
            }
            Gesture::Hold(fingers) => self.hold.get(&Fingers(fingers)),
        }
    }

    pub fn repeats(&self, gesture: Gesture) -> bool {
        self.binding(gesture).is_some_and(|binding| binding.repeat)
    }

    /// Every bound gesture with its binding, for `tygesture check`.
    pub fn bindings(&self) -> Vec<(Gesture, &Binding)> {
        let mut all = Vec::new();
        for &Fingers(f) in self.swipe.keys() {
            for dir in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
                all.push(Gesture::Swipe(f, dir));
            }
        }
        for &Fingers(f) in self.pinch.keys() {
            all.extend([Gesture::Pinch(f, Zoom::In), Gesture::Pinch(f, Zoom::Out)]);
        }
        for &Fingers(f) in self.rotate.keys() {
            all.extend([Gesture::Rotate(f, Spin::Cw), Gesture::Rotate(f, Spin::Ccw)]);
        }
        for &Fingers(f) in self.hold.keys() {
            all.push(Gesture::Hold(f));
        }
        all.into_iter()
            .filter_map(|g| self.binding(g).map(|b| (g, b)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_and_full_bindings() {
        let cfg: Config = ty_common::config::parse(
            r#"
thresholds: { swipe: 300 }
swipe:
  3:
    left: xdotool key alt+Right
  "4":
    up: { command: pactl set-sink-volume @DEFAULT_SINK@ +5%, repeat: true }
pinch:
  2: { in: zoom-out, out: zoom-in }
hold:
  4: rofi -show drun
"#,
        )
        .unwrap();
        assert_eq!(cfg.thresholds.swipe, 300.0);
        assert_eq!(cfg.thresholds.pinch, 0.2);
        assert!(cfg.tray);
        assert_eq!(
            cfg.binding(Gesture::Swipe(3, Dir::Left)).unwrap().command,
            "xdotool key alt+Right"
        );
        assert!(cfg.repeats(Gesture::Swipe(4, Dir::Up)));
        assert!(!cfg.repeats(Gesture::Swipe(3, Dir::Left)));
        assert!(cfg.binding(Gesture::Swipe(3, Dir::Right)).is_none());
        assert_eq!(
            cfg.binding(Gesture::Pinch(2, Zoom::In)).unwrap().command,
            "zoom-out"
        );
        assert!(cfg.binding(Gesture::Hold(4)).is_some());
        assert_eq!(cfg.bindings().len(), 5);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(ty_common::config::parse::<Config>("swipe: { 3: { sideways: x } }").is_err());
    }
}
