//! Startup and demo messages use the same D-Bus API as real notification clients.
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use gtk::glib::variant::ToVariant;
use gtk::{gio, glib};

use crate::bus;

struct Example {
    summary: &'static str,
    body: &'static str,
    icon: &'static str,
    urgency: u8,
    timeout: i32,
    actions: Vec<&'static str>,
    hints: HashMap<&'static str, glib::Variant>,
    replace_previous: bool,
}

impl Example {
    fn new(summary: &'static str, body: &'static str) -> Self {
        Self {
            summary,
            body,
            icon: "dialog-information",
            urgency: 1,
            timeout: 2500,
            actions: Vec::new(),
            hints: HashMap::new(),
            replace_previous: false,
        }
    }

    fn hint(mut self, key: &'static str, value: impl ToVariant) -> Self {
        self.hints.insert(key, value.to_variant());
        self
    }
}

fn examples(delay: Duration) -> VecDeque<Example> {
    let mut low = Example::new("Low urgency", "A quiet, low-priority notification.");
    low.urgency = 0;
    let mut critical = Example::new(
        "Critical urgency",
        "Critical border styling; this demo expires automatically.",
    );
    critical.urgency = 2;
    critical.icon = "dialog-error";
    let mut actions = Example::new(
        "Actions",
        "Click a button; the invoked action is printed in the terminal.",
    );
    actions.actions = vec!["default", "Open", "reply", "Reply"];
    let mut resident = Example::new(
        "Resident action",
        "Clicking Acknowledge keeps this card open until its timeout.",
    )
    .hint("resident", true);
    resident.actions = vec!["acknowledge", "Acknowledge"];
    let mut replacement = Example::new(
        "Replaced notification",
        "Updated the previous card using its notification ID.",
    );
    replacement.replace_previous = true;
    let mut sticky = Example::new(
        "Sticky notification",
        "No timeout: dismiss this card with × or tynotif close.",
    );
    sticky.timeout = 0;
    let mut sticky_critical = Example::new(
        "Critical, default timeout",
        "Critical notifications stay until dismissed by default. Demo complete.",
    );
    sticky_critical.urgency = 2;
    sticky_critical.icon = "dialog-warning";
    sticky_critical.timeout = -1;
    let mut examples = VecDeque::from(vec![
        low,
        Example::new("Normal urgency", "A normal desktop notification, with an application icon."),
        critical,
        Example::new("Long body", "This longer notification demonstrates word wrapping within the popup card.\nIt also includes a second paragraph, with Unicode: café — 日本語 — 🔔."),
        actions,
        resident,
        Example::new("Transient notification", "This notification will not be saved in history.").hint("transient", true),
        Example::new("Progress: 10%", "The next updates replace this card using a stack tag.")
            .hint("value", 10i32).hint("x-dunst-stack-tag", "tynotif-demo-progress"),
        Example::new("Progress: 50%", "Halfway there.")
            .hint("value", 50i32).hint("x-dunst-stack-tag", "tynotif-demo-progress"),
        Example::new("Progress: 100%", "Complete.")
            .hint("value", 100i32).hint("x-dunst-stack-tag", "tynotif-demo-progress"),
        Example::new("Replacement target", "The next demo step will update this card by ID."),
        replacement,
        sticky,
        sticky_critical,
    ]);
    // Keep replacement targets alive until the next step, even with a long delay.
    for example in &mut examples {
        if example.timeout > 0 {
            example.timeout = (delay.as_millis() + 1500).max(2500).min(i32::MAX as u128) as i32;
        }
    }
    examples
}

fn send(
    connection: &gio::DBusConnection,
    mut example: Example,
    previous_id: u32,
    done: impl FnOnce(u32) + 'static,
) {
    example
        .hints
        .insert("urgency", example.urgency.to_variant());
    let replaces = if example.replace_previous {
        previous_id
    } else {
        0
    };
    let parameters = (
        "tynotif",
        replaces,
        example.icon,
        example.summary,
        example.body,
        example.actions,
        example.hints,
        example.timeout,
    )
        .to_variant();
    // Target our unique connection, not whichever daemon might subsequently take over.
    let destination = connection.unique_name();
    connection.call(
        destination.as_deref(),
        bus::PATH,
        bus::NAME,
        "Notify",
        Some(&parameters),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        5000,
        gio::Cancellable::NONE,
        move |reply| {
            let id = match reply {
                Ok(value) => value.get::<(u32,)>().map(|(id,)| id).unwrap_or(0),
                Err(error) => {
                    eprintln!("Could not send tynotif example: {error}");
                    0
                }
            };
            done(id);
        },
    );
}

fn next(
    connection: gio::DBusConnection,
    mut examples: VecDeque<Example>,
    previous_id: u32,
    delay: Duration,
) {
    let Some(example) = examples.pop_front() else {
        println!("tynotif notification demo complete");
        return;
    };
    glib::timeout_add_local_once(delay, move || {
        let next_connection = connection.clone();
        println!("tynotif test: {}", example.summary);
        send(&connection, example, previous_id, move |id| {
            next(next_connection, examples, id, delay)
        });
    });
}

pub fn start(connection: &gio::DBusConnection, test: Option<Duration>) {
    if test.is_some() {
        connection.signal_subscribe(
            connection.unique_name().as_deref(),
            Some(bus::NAME),
            Some("ActionInvoked"),
            Some(bus::PATH),
            None,
            gio::DBusSignalFlags::NONE,
            |_, _, _, _, _, parameters| {
                if let Some((id, key)) = parameters.get::<(u32, String)>() {
                    println!("tynotif test action: notification {id}, action {key}");
                }
            },
        );
    }
    let mut startup = Example::new("tynotif is running", "Ready to receive notifications.")
        .hint("transient", true);
    startup.icon = "tynotif";
    startup.timeout = 3000;
    let connection_copy = connection.clone();
    send(connection, startup, 0, move |_| {
        println!("tynotif is running");
        if let Some(delay) = test {
            next(connection_copy, examples(delay), 0, delay);
        }
    });
}
