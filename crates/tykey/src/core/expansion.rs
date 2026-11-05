use anyhow::{bail, Result};
use chrono::Local;
use std::collections::HashMap;
use std::process::Command;

use crate::io::output::SpecialKey;

#[derive(Debug, Clone)]
pub enum OutputAction {
    Text(String),
    Key(SpecialKey),
    SleepMs(u64),
    MoveCaret(i64),
}

pub fn render_template_macros(input: &str, globals: &HashMap<String, String>) -> Result<String> {
    render_template_macros_internal(input, globals, &mut Vec::new())
}

pub fn parse_expansion_actions(
    input: &str,
    globals: &HashMap<String, String>,
) -> Result<Vec<OutputAction>> {
    let templated = render_template_macros(input, globals)?;
    parse_action_macros_only(&templated)
}

fn render_template_macros_internal(
    input: &str,
    globals: &HashMap<String, String>,
    resolving_stack: &mut Vec<String>,
) -> Result<String> {
    let mut rendered = String::with_capacity(input.len());
    let mut i = 0usize;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        if starts_with_at(bytes, i, b"{{") {
            let end = find_macro_end(input, i + 2)
                .ok_or_else(|| anyhow::anyhow!("unclosed macro starting at byte {}", i))?;
            let body = input[i + 2..end].trim();

            if let Some((name, value)) = body.split_once(':') {
                if is_template_macro_with_argument(name) {
                    rendered.push_str(&render_template_macro_with_argument(
                        name.trim(),
                        value.trim(),
                        globals,
                        resolving_stack,
                    )?);
                } else {
                    rendered.push_str(&input[i..end + 2]);
                }
            } else {
                rendered.push_str(&render_template_macro(body, globals, resolving_stack)?);
            }

            i = end + 2;
            continue;
        }

        let ch = input[i..].chars().next().expect("char exists");
        rendered.push(ch);
        i += ch.len_utf8();
    }

    Ok(rendered)
}

fn parse_action_macros_only(input: &str) -> Result<Vec<OutputAction>> {
    let mut actions = Vec::new();
    let mut text_buf = String::new();
    let mut i = 0usize;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        if starts_with_at(bytes, i, b"{{") {
            if !text_buf.is_empty() {
                actions.push(OutputAction::Text(std::mem::take(&mut text_buf)));
            }

            let end = find_macro_end(input, i + 2)
                .ok_or_else(|| anyhow::anyhow!("unclosed macro starting at byte {}", i))?;
            let body = &input[i + 2..end];
            if body.contains(':') {
                actions.push(parse_action_macro(body.trim())?);
            } else {
                text_buf.push_str(&input[i..end + 2]);
            }
            i = end + 2;
            continue;
        }

        text_buf.push(input[i..].chars().next().expect("char exists"));
        i += input[i..].chars().next().expect("char exists").len_utf8();
    }

    if !text_buf.is_empty() {
        actions.push(OutputAction::Text(text_buf));
    }

    Ok(actions)
}

fn starts_with_at(haystack: &[u8], index: usize, needle: &[u8]) -> bool {
    haystack.get(index..index + needle.len()) == Some(needle)
}

fn find_macro_end(input: &str, start: usize) -> Option<usize> {
    input[start..].find("}}").map(|offset| start + offset)
}

fn parse_action_macro(body: &str) -> Result<OutputAction> {
    if let Some((name, value)) = body.split_once(':') {
        let name = name.trim().to_ascii_uppercase();
        let value = value.trim();

        return match name.as_str() {
            "KEY" => Ok(OutputAction::Key(parse_special_key(value)?)),
            "SLEEP_MS" => {
                let ms: u64 = value.parse()?;
                Ok(OutputAction::SleepMs(ms))
            }
            "MOVE_CARET" | "CARET_MOVE" => {
                let amount: i64 = value.parse()?;
                Ok(OutputAction::MoveCaret(amount))
            }
            _ => bail!("unsupported macro: '{name}'"),
        };
    }

    bail!("unsupported macro: '{body}'")
}

fn render_template_macro(
    name: &str,
    globals: &HashMap<String, String>,
    resolving_stack: &mut Vec<String>,
) -> Result<String> {
    let now = Local::now();
    let normalized_name = name.trim().to_ascii_uppercase();
    let rendered = match normalized_name.as_str() {
        "DATETIME" => now.format("%Y-%m-%d %H:%M:%S").to_string(),
        "DATE" => now.format("%Y-%m-%d").to_string(),
        "TIME" => now.format("%H:%M:%S").to_string(),
        _ => resolve_global_template_macro(&normalized_name, globals, resolving_stack)?,
    };
    Ok(rendered)
}

fn resolve_global_template_macro(
    name: &str,
    globals: &HashMap<String, String>,
    resolving_stack: &mut Vec<String>,
) -> Result<String> {
    let Some(value) = lookup_global_macro_case_insensitive(globals, name) else {
        bail!("unsupported macro: '{name}'");
    };

    if resolving_stack.iter().any(|existing| existing == name) {
        let mut chain = resolving_stack.clone();
        chain.push(name.to_string());
        bail!("global macro cycle detected: {}", chain.join(" -> "));
    }

    resolving_stack.push(name.to_string());
    let rendered = render_template_macros_internal(value, globals, resolving_stack)?;
    resolving_stack.pop();
    Ok(rendered)
}

fn lookup_global_macro_case_insensitive<'a>(
    globals: &'a HashMap<String, String>,
    name: &str,
) -> Option<&'a str> {
    for (global_name, value) in globals {
        if global_name.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

fn is_template_macro_with_argument(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "CMD" | "COMMAND" | "EMOJI"
    )
}

fn render_template_macro_with_argument(
    name: &str,
    value: &str,
    globals: &HashMap<String, String>,
    resolving_stack: &mut Vec<String>,
) -> Result<String> {
    let normalized = name.to_ascii_uppercase();
    match normalized.as_str() {
        "CMD" | "COMMAND" => run_linux_command_macro(value, globals, resolving_stack),
        "EMOJI" => render_emoji_macro(value, globals, resolving_stack),
        _ => bail!("unsupported macro: '{normalized}'"),
    }
}

fn render_emoji_macro(
    shortcode: &str,
    globals: &HashMap<String, String>,
    resolving_stack: &mut Vec<String>,
) -> Result<String> {
    let rendered_shortcode = render_template_macros_internal(shortcode, globals, resolving_stack)?;
    let normalized_shortcode = rendered_shortcode
        .trim()
        .trim_matches(':')
        .to_ascii_lowercase();
    let lookup_candidates = [
        normalized_shortcode.clone(),
        normalized_shortcode.replace('-', "_"),
        normalized_shortcode.replace('-', ""),
    ];
    let emoji = lookup_candidates
        .iter()
        .find_map(|candidate| emojis::get_by_shortcode(candidate));
    let Some(emoji) = emoji else {
        bail!("unknown emoji shortcode: '{normalized_shortcode}'");
    };

    Ok(emoji.as_str().to_string())
}

fn run_linux_command_macro(
    command: &str,
    globals: &HashMap<String, String>,
    resolving_stack: &mut Vec<String>,
) -> Result<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (command, globals, resolving_stack);
        bail!("CMD macro is only supported on Linux");
    }

    #[cfg(target_os = "linux")]
    {
        let rendered_command = render_template_macros_internal(command, globals, resolving_stack)?;
        let output = Command::new("sh")
            .arg("-c")
            .arg(&rendered_command)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "CMD macro command failed (status: {}): {}",
                output.status.code().map_or_else(
                    || "terminated by signal".to_string(),
                    |code| code.to_string()
                ),
                stderr.trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim_end_matches(['\r', '\n']).to_string())
    }
}

#[cfg(test)]
fn is_valid_for_format(value: &str, format: &str) -> bool {
    chrono::NaiveDateTime::parse_from_str(value, format).is_ok()
        || chrono::NaiveDate::parse_from_str(value, format).is_ok()
        || chrono::NaiveTime::parse_from_str(value, format).is_ok()
}

fn parse_special_key(name: &str) -> Result<SpecialKey> {
    let key = match name.to_ascii_uppercase().as_str() {
        "ENTER" | "RETURN" => SpecialKey::Enter,
        "TAB" => SpecialKey::Tab,
        "ESC" | "ESCAPE" => SpecialKey::Escape,
        "BACKSPACE" => SpecialKey::Backspace,
        "SPACE" => SpecialKey::Space,
        "LEFT" => SpecialKey::Left,
        "RIGHT" => SpecialKey::Right,
        "UP" => SpecialKey::Up,
        "DOWN" => SpecialKey::Down,
        "HOME" => SpecialKey::Home,
        "END" => SpecialKey::End,
        "DELETE" => SpecialKey::Delete,
        "PAGEUP" => SpecialKey::PageUp,
        "PAGEDOWN" => SpecialKey::PageDown,
        "F1" => SpecialKey::F1,
        "F2" => SpecialKey::F2,
        "F3" => SpecialKey::F3,
        "F4" => SpecialKey::F4,
        "F5" => SpecialKey::F5,
        "F6" => SpecialKey::F6,
        "F7" => SpecialKey::F7,
        "F8" => SpecialKey::F8,
        "F9" => SpecialKey::F9,
        "F10" => SpecialKey::F10,
        "F11" => SpecialKey::F11,
        "F12" => SpecialKey::F12,
        other => bail!("unknown special key in macro: {other}"),
    };
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::{
        is_valid_for_format, parse_expansion_actions, render_template_macros, OutputAction,
    };
    use crate::io::output::SpecialKey;
    use std::collections::HashMap;

    fn no_globals() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn parses_plain_text_as_single_action() {
        let actions =
            parse_expansion_actions("hello world", &no_globals()).expect("parsing should succeed");

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            OutputAction::Text(text) => assert_eq!(text, "hello world"),
            _ => panic!("expected text action"),
        }
    }

    #[test]
    fn parses_mixed_text_and_macros() {
        let actions = parse_expansion_actions("Hi{{KEY:ENTER}}{{SLEEP_MS:50}}there", &no_globals())
            .expect("parsing should succeed");

        assert_eq!(actions.len(), 4);
        match &actions[0] {
            OutputAction::Text(text) => assert_eq!(text, "Hi"),
            _ => panic!("expected first text action"),
        }
        match actions[1] {
            OutputAction::Key(SpecialKey::Enter) => {}
            _ => panic!("expected enter key action"),
        }
        match actions[2] {
            OutputAction::SleepMs(50) => {}
            _ => panic!("expected sleep action"),
        }
        match &actions[3] {
            OutputAction::Text(text) => assert_eq!(text, "there"),
            _ => panic!("expected trailing text action"),
        }
    }

    #[test]
    fn parses_move_caret_macro() {
        let actions = parse_expansion_actions("x{{MOVE_CARET:-3}}y", &no_globals())
            .expect("parsing should succeed");

        assert_eq!(actions.len(), 3);
        match &actions[0] {
            OutputAction::Text(text) => assert_eq!(text, "x"),
            _ => panic!("expected first text action"),
        }
        match actions[1] {
            OutputAction::MoveCaret(-3) => {}
            _ => panic!("expected move caret action"),
        }
        match &actions[2] {
            OutputAction::Text(text) => assert_eq!(text, "y"),
            _ => panic!("expected trailing text action"),
        }
    }

    #[test]
    fn parses_caret_move_alias() {
        let actions = parse_expansion_actions("{{CARET_MOVE:2}}", &no_globals())
            .expect("parsing should succeed");

        assert_eq!(actions.len(), 1);
        match actions[0] {
            OutputAction::MoveCaret(2) => {}
            _ => panic!("expected move caret action"),
        }
    }

    #[test]
    fn parses_datetime_macro_in_expansion() {
        let actions = parse_expansion_actions("Today: {{DATE}} {{TIME}}", &no_globals())
            .expect("parsing should succeed");

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            OutputAction::Text(text) => {
                assert!(text.starts_with("Today: "));
                let suffix = &text["Today: ".len()..];
                let (date, time) = suffix
                    .split_once(' ')
                    .expect("text should contain date and time");
                assert!(is_valid_for_format(date, "%Y-%m-%d"));
                assert!(is_valid_for_format(time, "%H:%M:%S"));
            }
            _ => panic!("expected rendered text action"),
        }
    }

    #[test]
    fn renders_template_macros_for_snippets() {
        let rendered = render_template_macros("Now: {{DATETIME}}", &no_globals())
            .expect("render should succeed");
        assert!(rendered.starts_with("Now: "));
        assert!(is_valid_for_format(
            &rendered["Now: ".len()..],
            "%Y-%m-%d %H:%M:%S"
        ));
    }

    #[test]
    fn render_templates_keeps_action_macros_literal() {
        let rendered = render_template_macros("Keep {{KEY:ENTER}} and {{DATE}}", &no_globals())
            .expect("render should work");
        assert!(rendered.contains("{{KEY:ENTER}}"));
        assert!(!rendered.contains("{{DATE}}"));
    }

    #[test]
    fn rejects_unclosed_macro() {
        let err = parse_expansion_actions("x{{KEY:ENTER", &no_globals())
            .expect_err("unclosed macro should return error");
        assert!(err.to_string().contains("unclosed macro"));
    }

    #[test]
    fn renders_global_template_macro_with_nested_macros() {
        let mut globals = HashMap::new();
        globals.insert("GREETING".to_string(), "Hello".to_string());
        globals.insert(
            "SIGNOFF".to_string(),
            "{{GREETING}}, Tyler on {{DATE}}".to_string(),
        );

        let rendered =
            render_template_macros("Msg: {{SIGNOFF}}", &globals).expect("render should succeed");
        assert!(rendered.starts_with("Msg: Hello, Tyler on "));
    }

    #[test]
    fn parses_actions_from_global_template_expansion() {
        let mut globals = HashMap::new();
        globals.insert("SIGNATURE".to_string(), "Thanks{{KEY:ENTER}}".to_string());

        let actions =
            parse_expansion_actions("{{SIGNATURE}}", &globals).expect("parsing should succeed");
        assert_eq!(actions.len(), 2);
        match &actions[0] {
            OutputAction::Text(text) => assert_eq!(text, "Thanks"),
            _ => panic!("expected text action"),
        }
        match actions[1] {
            OutputAction::Key(SpecialKey::Enter) => {}
            _ => panic!("expected enter key action"),
        }
    }

    #[test]
    fn renders_cmd_macro_output() {
        let rendered = render_template_macros("{{CMD:printf hello}}", &no_globals())
            .expect("command macro should render");
        assert_eq!(rendered, "hello");
    }

    #[test]
    fn renders_emoji_macro_output() {
        let rendered = render_template_macros("Ship it {{EMOJI:rocket}}", &no_globals())
            .expect("emoji macro should render");
        assert_eq!(rendered, "Ship it 🚀");
    }

    #[test]
    fn renders_emoji_macro_with_dash_shortcode() {
        let rendered = render_template_macros("{{EMOJI:thumbs-up}}", &no_globals())
            .expect("emoji macro should render");
        assert_eq!(rendered, "👍");
    }

    #[test]
    fn rejects_unknown_emoji_shortcode() {
        let err = render_template_macros("{{EMOJI:not-a-real-emoji}}", &no_globals())
            .expect_err("unknown emoji shortcode should fail");
        assert!(err.to_string().contains("unknown emoji shortcode"));
    }

    #[test]
    fn rejects_global_macro_cycles() {
        let mut globals = HashMap::new();
        globals.insert("A".to_string(), "{{B}}".to_string());
        globals.insert("B".to_string(), "{{A}}".to_string());

        let err = render_template_macros("{{A}}", &globals).expect_err("cycle should fail");
        assert!(err.to_string().contains("cycle"));
    }
}
