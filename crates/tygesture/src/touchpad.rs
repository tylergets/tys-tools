//! Reads gesture events straight from libinput (via udev on seat0).
//!
//! libinput doesn't grab the devices, so this runs alongside X or the compositor. It needs
//! read access to `/dev/input/event*`, which normally means being in the `input` group.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use input::event::gesture::{
    GestureEndEvent, GestureEventCoordinates, GestureEventTrait, GestureHoldEvent,
    GesturePinchEvent, GesturePinchEventTrait, GestureSwipeEvent,
};
use input::event::{DeviceEvent, EventTrait, GestureEvent};
use input::{DeviceCapability, Event, Libinput, LibinputInterface};

use crate::gesture::Raw;

struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let access = flags & libc::O_ACCMODE;
        OpenOptions::new()
            .custom_flags(flags)
            .read(access == libc::O_RDONLY || access == libc::O_RDWR)
            .write(access == libc::O_WRONLY || access == libc::O_RDWR)
            .open(path)
            .map(OwnedFd::from)
            .map_err(|err| -err.raw_os_error().unwrap_or(libc::EIO))
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(File::from(fd));
    }
}

/// What the event loop hands to its callback.
pub enum Item {
    Gesture(Raw),
    /// A gesture-capable device appeared (`true`) or went away (`false`).
    Touchpad(String, bool),
}

/// Run libinput's event loop forever, calling `handle` for each gesture or touchpad event.
pub fn run(mut handle: impl FnMut(Item)) -> Result<()> {
    let mut libinput = Libinput::new_with_udev(Interface);
    libinput
        .udev_assign_seat("seat0")
        .map_err(|()| anyhow!("libinput could not assign seat0"))?;

    let mut touchpads = 0usize;
    let mut first = true;
    loop {
        libinput.dispatch().context("libinput dispatch failed")?;
        for event in &mut libinput {
            if let Some(item) = convert(&event) {
                if let Item::Touchpad(_, added) = item {
                    touchpads = if added {
                        touchpads + 1
                    } else {
                        touchpads.saturating_sub(1)
                    };
                }
                handle(item);
            }
        }
        if first {
            first = false;
            if touchpads == 0 {
                eprintln!(
                    "tygesture: no touchpad with gesture support found. libinput needs read \
                     access to /dev/input/event* (usually the `input` group); still waiting for one"
                );
            }
        }
        wait_readable(&libinput)?;
    }
}

fn wait_readable(libinput: &Libinput) -> Result<()> {
    let mut fd = libc::pollfd {
        fd: libinput.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: one valid pollfd for the duration of the call.
        if unsafe { libc::poll(&mut fd, 1, -1) } >= 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err).context("poll on libinput fd failed");
        }
    }
}

fn convert(event: &Event) -> Option<Item> {
    let fingers = |n: i32| n.max(0) as u32;
    let raw = match event {
        Event::Device(device) => {
            let added = matches!(device, DeviceEvent::Added(_));
            let device = device.device();
            if !device.has_capability(DeviceCapability::Gesture) {
                return None;
            }
            return Some(Item::Touchpad(device.name().into_owned(), added));
        }
        Event::Gesture(GestureEvent::Swipe(swipe)) => match swipe {
            GestureSwipeEvent::Begin(e) => Raw::SwipeBegin(fingers(e.finger_count())),
            GestureSwipeEvent::Update(e) => Raw::SwipeUpdate {
                dx: e.dx_unaccelerated(),
                dy: e.dy_unaccelerated(),
            },
            GestureSwipeEvent::End(e) => Raw::SwipeEnd {
                cancelled: e.cancelled(),
            },
            _ => return None,
        },
        Event::Gesture(GestureEvent::Pinch(pinch)) => match pinch {
            GesturePinchEvent::Begin(e) => Raw::PinchBegin(fingers(e.finger_count())),
            GesturePinchEvent::Update(e) => Raw::PinchUpdate {
                scale: e.scale(),
                angle: e.angle_delta(),
            },
            GesturePinchEvent::End(e) => Raw::PinchEnd {
                cancelled: e.cancelled(),
            },
            _ => return None,
        },
        Event::Gesture(GestureEvent::Hold(hold)) => match hold {
            GestureHoldEvent::Begin(e) => Raw::HoldBegin(fingers(e.finger_count())),
            GestureHoldEvent::End(e) => Raw::HoldEnd {
                cancelled: e.cancelled(),
            },
            _ => return None,
        },
        _ => return None,
    };
    Some(Item::Gesture(raw))
}
