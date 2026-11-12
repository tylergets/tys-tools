//! Notification text is untrusted markup, never a format string or shell command.
use crate::{config::Markup, model::Notification};
use gtk::{glib, pango};

pub fn escape(text: &str) -> String {
    glib::markup_escape_text(text).to_string()
}

/// Walk tags without treating a quoted '>' as the end of an attribute.
fn tags(text: &str, mut emit: impl FnMut(&str, bool)) {
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        emit(&rest[..start], false);
        let tail = &rest[start..];
        let mut quote = None;
        let end = tail.char_indices().skip(1).find_map(|(i, c)| {
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if c == '\'' || c == '"' {
                quote = Some(c);
            } else if c == '>' {
                return Some(i);
            }
            None
        });
        let Some(end) = end else {
            emit(tail, false);
            return;
        };
        emit(&tail[..=end], true);
        rest = &tail[end + 1..];
    }
    emit(rest, false);
}

fn tag_name(tag: &str) -> &str {
    tag.trim_start_matches('<')
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
}

fn attribute(tag: &str, key: &str) -> Option<String> {
    let mut rest = tag.trim_start_matches('<');
    rest = rest.get(rest.find(char::is_whitespace)?..)?;
    loop {
        rest = rest.trim_start();
        let end = rest.find(|c: char| c.is_whitespace() || c == '=')?;
        let name = &rest[..end];
        rest = rest[end..].trim_start().strip_prefix('=')?.trim_start();
        let quote = rest.chars().next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        rest = &rest[1..];
        let end = rest.find(quote)?;
        let value = &rest[..end];
        rest = &rest[end + 1..];
        if name == key {
            return Some(decode(value));
        }
    }
}

fn decode(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut candidate = chars.clone();
        let entity: String = candidate
            .by_ref()
            .take(16)
            .take_while(|c| *c != ';')
            .collect();
        let decoded = match entity.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            value => value
                .strip_prefix("#x")
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                .or_else(|| value.strip_prefix('#').and_then(|s| s.parse().ok()))
                .and_then(char::from_u32),
        }
        .filter(|c| {
            matches!(*c, '\t' | '\n' | '\r') || (*c >= ' ' && *c != '\u{fffe}' && *c != '\u{ffff}')
        });
        // Verify the terminator rather than consuming an unfinished entity.
        if let Some(decoded) =
            decoded.filter(|_| chars.clone().nth(entity.chars().count()) == Some(';'))
        {
            for _ in 0..=entity.chars().count() {
                chars.next();
            }
            out.push(decoded);
        } else {
            out.push('&');
        }
    }
    out
}

pub fn plain(text: &str) -> String {
    let mut out = String::new();
    tags(text, |part, tag| {
        if !tag {
            out.push_str(part);
        } else if tag_name(part).is_empty() {
            out.push_str(part);
        } else if tag_name(part) == "br" {
            out.push('\n');
        } else if tag_name(part) == "img" {
            out.push_str(&escape(&attribute(part, "alt").unwrap_or_default()));
        }
    });
    decode(&out)
}

pub fn render(text: &str, mode: Markup) -> String {
    match mode {
        Markup::No => escape(text),
        Markup::Strip => escape(&plain(text)),
        Markup::Full => {
            let mut out = String::new();
            tags(text, |part, tag| {
                if !tag {
                    out.push_str(part);
                    return;
                }
                match tag_name(part) {
                    // URLs are offered through the context menu. Pango itself has no <a>.
                    "a" => {}
                    "br" => out.push('\n'),
                    "img" => out.push_str(&escape(&attribute(part, "alt").unwrap_or_default())),
                    _ => out.push_str(part),
                }
            });
            if pango::parse_markup(&out, '\0').is_ok() {
                out
            } else {
                escape(&plain(text))
            }
        }
    }
}

pub fn urls(n: &Notification) -> Vec<String> {
    let mut urls = Vec::new();
    let mut add = |url: String| {
        if ["https://", "http://", "ftp://", "file://", "mailto:"]
            .iter()
            .any(|s| url.starts_with(s))
            && !urls.contains(&url)
        {
            urls.push(url);
        }
    };
    for text in [&n.summary, &n.body] {
        tags(text, |part, tag| {
            if tag && tag_name(part) == "a" {
                if let Some(url) = attribute(part, "href") {
                    add(url);
                }
            }
        });
        let text = plain(text);
        let mut offset = 0;
        while offset < text.len() {
            let tail = &text[offset..];
            if ["https://", "http://", "ftp://", "file://", "mailto:"]
                .iter()
                .any(|s| tail.starts_with(s))
            {
                let end = tail
                    .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
                    .unwrap_or(tail.len());
                let mut url = tail[..end]
                    .trim_end_matches(['.', ',', ';', '!', '?'])
                    .to_string();
                for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
                    while url.ends_with(close)
                        && url.matches(close).count() > url.matches(open).count()
                    {
                        url.pop();
                    }
                }
                add(url);
                offset += end;
            } else {
                offset += tail.chars().next().unwrap().len_utf8();
            }
        }
    }
    urls
}

pub fn formatted(n: &Notification, template: &str, mode: Markup) -> String {
    let template = template.replace("\\n", "\n").replace("\\t", "\t");
    let mut chars = template.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let value = match chars.next() {
            Some('a') => escape(&n.app),
            Some('s') => render(&n.summary, mode),
            Some('b') => render(&n.body, mode),
            Some('c') => escape(&n.category),
            Some('S') => escape(&n.tag),
            Some('i') => escape(&n.icon),
            Some('I') => escape(n.icon.rsplit('/').next().unwrap_or("")),
            Some('p') => n.progress.map(|v| format!("[{v:3}%]")).unwrap_or_default(),
            Some('n') => n.progress.map(|v| v.to_string()).unwrap_or_default(),
            Some('%') => "%".into(),
            Some(c) => format!("%{c}"),
            None => "%".into(),
        };
        out.push_str(&value);
    }
    if pango::parse_markup(&out, '\0').is_ok() {
        out
    } else {
        escape(&plain(&out))
    }
}

/// Normalize only hexadecimal color hints, so they cannot inject GTK CSS.
pub fn color(value: &str) -> Option<String> {
    let hex = value.strip_prefix('#')?;
    if ![3, 4, 6, 8].contains(&hex.len()) || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let full = if hex.len() <= 4 {
        hex.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        hex.to_string()
    };
    let r = u8::from_str_radix(&full[0..2], 16).ok()?;
    let g = u8::from_str_radix(&full[2..4], 16).ok()?;
    let b = u8::from_str_radix(&full[4..6], 16).ok()?;
    let alpha = if full.len() == 8 {
        f64::from(u8::from_str_radix(&full[6..8], 16).ok()?) / 255.0
    } else {
        1.0
    };
    Some(format!("rgba({r},{g},{b},{alpha})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markup_modes_and_malformed_input() {
        assert_eq!(
            render("<b>bold &amp; nice</b>", Markup::Full),
            "<b>bold &amp; nice</b>"
        );
        assert_eq!(render("<b>bold</b>", Markup::Strip), "bold");
        assert_eq!(render("<b>bold</b>", Markup::No), "&lt;b&gt;bold&lt;/b&gt;");
        assert_eq!(render("<b>broken", Markup::Full), "broken");
        assert_eq!(render("a & b", Markup::Full), "a &amp; b");
        assert_eq!(
            plain("<a href='https://example.org/?a=1&amp;b=2'>link</a><br/>next"),
            "link\nnext"
        );
        assert_eq!(
            attribute("<a title='x > y' href='https://example.org'>", "href").as_deref(),
            Some("https://example.org")
        );
    }
    #[test]
    fn colors_reject_css_and_accept_alpha() {
        assert_eq!(color("#f00"), Some("rgba(255,0,0,1)".into()));
        assert!(color("#abcd").is_some());
        assert!(color("red; background: blue").is_none());
    }
    #[test]
    fn template_substitution_is_single_pass_and_escapes_metadata() {
        let mut n = crate::model::tests::notification();
        n.app = "<untrusted>&%b".into();
        n.summary = "<b>title</b>".into();
        n.body = "100% %a".into();
        n.progress = Some(42);
        assert_eq!(
            formatted(&n, "%a\\n%s %b %n %%", Markup::Full),
            "&lt;untrusted&gt;&amp;%b\n<b>title</b> 100% %a 42 %"
        );
        assert_eq!(
            formatted(&n, "<b>%s</b>", Markup::No),
            "<b>&lt;b&gt;title&lt;/b&gt;</b>"
        );
    }
    #[test]
    fn extracts_deduplicated_links_without_losing_balanced_parentheses() {
        let mut n = crate::model::tests::notification();
        n.body = "<a href='https://example.org/?a=1&amp;b=2'>link</a> https://example.org/?a=1&amp;b=2 and (https://example.org/a_(b)). <a href='javascript:alert(1)'>bad</a>".into();
        assert_eq!(
            urls(&n),
            vec!["https://example.org/?a=1&b=2", "https://example.org/a_(b)"]
        );
        n.body = "https://example.org/?redirect=https://other.org".into();
        assert_eq!(
            urls(&n),
            vec!["https://example.org/?redirect=https://other.org"]
        );
    }
}
