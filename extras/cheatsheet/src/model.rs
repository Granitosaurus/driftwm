//! Bindings as `driftwm msg --json bindings` reports them, turned into the
//! grouped, human-worded rows the cheatsheet shows.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct BindingsReply {
    #[serde(rename = "Bindings")]
    bindings: Bindings,
}

#[derive(Debug, Deserialize)]
pub struct Bindings {
    pub mod_key: String,
    pub keys: Vec<Binding>,
    pub taps: Vec<Binding>,
    pub mouse: Vec<Binding>,
}

#[derive(Debug, Deserialize)]
pub struct Binding {
    pub trigger: String,
    #[serde(default)]
    pub context: Option<String>,
    pub action: String,
}

/// Parse the `--json` reply, surfacing the compositor's own error text.
pub fn parse_reply(json: &str) -> Result<Bindings, String> {
    let reply: Result<BindingsReply, String> =
        serde_json::from_str(json).map_err(|e| format!("unexpected reply from driftwm: {e}"))?;
    reply.map(|r| r.bindings)
}

/// Section order on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Apps,
    Windows,
    Navigation,
    Zoom,
    Bookmarks,
    Session,
    Mouse,
    Taps,
    Other,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Apps => "Apps",
            Group::Windows => "Windows",
            Group::Navigation => "Navigation",
            Group::Zoom => "Zoom",
            Group::Bookmarks => "Bookmarks",
            Group::Session => "Session",
            Group::Mouse => "Mouse",
            Group::Taps => "Modifier taps",
            Group::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub group: Group,
    /// The trigger, spelled for reading: `Super + Return`.
    pub keys: String,
    pub what: String,
    /// The action as written in the config.
    pub raw: String,
}

impl Entry {
    fn haystack(&self) -> String {
        format!(
            "{} {} {} {}",
            self.keys,
            self.what,
            self.raw,
            self.group.title()
        )
        .to_lowercase()
    }
}

/// Every binding as a sorted row list, keys first, then mouse, then taps.
pub fn entries(bindings: &Bindings) -> Vec<Entry> {
    let mut out = Vec::new();
    for b in &bindings.keys {
        let (group, what) = describe(&b.action);
        out.push(Entry {
            group,
            keys: pretty_trigger(&b.trigger, pretty_key_part),
            what,
            raw: b.action.clone(),
        });
    }
    for b in &bindings.mouse {
        let (_, what) = describe(&b.action);
        let what = match b.context.as_deref() {
            Some("on-window") => format!("{what} (on a window)"),
            Some("on-canvas") => format!("{what} (on empty canvas)"),
            _ => what,
        };
        out.push(Entry {
            group: Group::Mouse,
            keys: pretty_trigger(&b.trigger, pretty_mouse_part),
            what,
            raw: b.action.clone(),
        });
    }
    for b in &bindings.taps {
        let (_, what) = describe(&b.action);
        out.push(Entry {
            group: Group::Taps,
            keys: format!("{} (tap)", pretty_trigger(&b.trigger, pretty_key_part)),
            what,
            raw: b.action.clone(),
        });
    }
    out.sort_by(|a, b| (a.group, &a.what, &a.keys).cmp(&(b.group, &b.what, &b.keys)));
    merge_same_action(out)
}

/// Fold rows that do the same thing into one, so `Super + Left` and `Super + H`
/// bound to the same action read as `Super + Left / Super + H`. Expects `rows`
/// sorted so such rows are adjacent.
fn merge_same_action(rows: Vec<Entry>) -> Vec<Entry> {
    let mut merged: Vec<Entry> = Vec::new();
    for row in rows {
        match merged.last_mut() {
            Some(last)
                if (last.group, &last.what, &last.raw) == (row.group, &row.what, &row.raw) =>
            {
                last.keys.push_str(" / ");
                last.keys.push_str(&row.keys);
            }
            _ => merged.push(row),
        }
    }
    merged
}

/// A command line shortened for reading: absolute paths reduced to their file
/// name, and a leading `bash`/`sh` dropped when it only runs a script.
fn summarize_command(command: &str) -> String {
    let mut tokens: Vec<&str> = command.split_whitespace().collect();
    if matches!(tokens.first(), Some(&("bash" | "sh")))
        && tokens.get(1).is_some_and(|t| !t.starts_with('-'))
    {
        tokens.remove(0);
    }
    tokens
        .iter()
        .map(|t| {
            if t.starts_with('/') {
                t.rsplit('/').next().unwrap_or(t)
            } else {
                t
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Group and plain-English wording for a config action string. Unknown actions
/// fall back to the raw string, so a new compositor action still shows up.
fn describe(action: &str) -> (Group, String) {
    let (name, arg) = action
        .split_once(' ')
        .map_or((action, ""), |(n, a)| (n, a.trim()));
    let with_arg = |verb: &str| {
        if arg.is_empty() {
            verb.to_owned()
        } else {
            format!("{verb} {arg}")
        }
    };
    let (group, what) = match name {
        "exec-terminal" => (Group::Apps, "Open a terminal".to_owned()),
        "exec-launcher" => (Group::Apps, "Open the app launcher".to_owned()),
        "exec" | "spawn" => (Group::Apps, format!("Run {}", summarize_command(arg))),
        "close-window" => (Group::Windows, "Close window".to_owned()),
        "suspend-window" => (
            Group::Windows,
            "Close window, leaving a restorable stand-in".to_owned(),
        ),
        "nudge-window" => (Group::Windows, with_arg("Nudge window")),
        "move-to-snap" => (Group::Windows, with_arg("Move window to next snap stop")),
        "grow-window" => (Group::Windows, format!("Push window's {arg} edge out")),
        "shrink-window" => (Group::Windows, format!("Pull window's {arg} edge in")),
        "resize-to-snap" => (Group::Windows, with_arg("Resize window to snap stop")),
        "toggle-fullscreen" => (Group::Windows, "Toggle fullscreen".to_owned()),
        "fit-window" => (Group::Windows, "Maximize window".to_owned()),
        "fit-window-snapped" => (
            Group::Windows,
            "Maximize window with its snapped neighbours".to_owned(),
        ),
        "fill-window" => (
            Group::Windows,
            "Grow window into the free space around it".to_owned(),
        ),
        "toggle-pin-to-screen" => (Group::Windows, "Pin window to the screen".to_owned()),
        "send-to-output" => (Group::Windows, format!("Send window to the {arg} monitor")),
        "move-window" => (Group::Windows, "Move window".to_owned()),
        "move-snapped-windows" => (
            Group::Windows,
            "Move window with its snapped neighbours".to_owned(),
        ),
        "resize-window" => (Group::Windows, "Resize window".to_owned()),
        "resize-window-snapped" => (
            Group::Windows,
            "Resize window with its snapped neighbours".to_owned(),
        ),
        "pan-viewport" if arg.is_empty() => (Group::Navigation, "Pan the view".to_owned()),
        "pan-viewport" => (Group::Navigation, with_arg("Pan the view")),
        "center-window" => (Group::Navigation, "Center window".to_owned()),
        "center-nearest" => (Group::Navigation, with_arg("Center the nearest window")),
        "focus-nearest" => (Group::Navigation, with_arg("Focus the nearest window")),
        "focus-center" => (
            Group::Navigation,
            "Focus the window under the pointer".to_owned(),
        ),
        "cycle-windows" => (Group::Navigation, with_arg("Cycle windows")),
        "home-toggle" => (Group::Navigation, "Jump to the origin and back".to_owned()),
        "send-cursor-to-output" => (
            Group::Navigation,
            format!("Send cursor to the {arg} monitor"),
        ),
        "zoom" => (Group::Zoom, "Zoom".to_owned()),
        "zoom-in" => (Group::Zoom, "Zoom in".to_owned()),
        "zoom-out" => (Group::Zoom, "Zoom out".to_owned()),
        "zoom-reset" => (Group::Zoom, "Reset zoom".to_owned()),
        "zoom-to-fit" => (Group::Zoom, "Zoom out to show every window".to_owned()),
        "zoom-to-fit-snapped" => (
            Group::Zoom,
            "Zoom to the focused window's snapped group".to_owned(),
        ),
        "go-to-bookmark" => (Group::Bookmarks, with_arg("Go to bookmark")),
        "set-bookmark" => (Group::Bookmarks, with_arg("Save the view as bookmark")),
        "move-to-bookmark" => (Group::Bookmarks, with_arg("Move window to bookmark")),
        "switch-layout" => (Group::Session, with_arg("Switch keyboard layout:")),
        "reload-config" => (Group::Session, "Reload the config".to_owned()),
        "toggle-cursor-pan" => (Group::Session, "Toggle edge panning".to_owned()),
        "set-trackpad" => (Group::Session, with_arg("Trackpad:")),
        "quit" => (Group::Session, "Quit driftwm".to_owned()),
        _ => (Group::Other, action.to_owned()),
    };
    (group, what)
}

fn pretty_trigger(trigger: &str, part: fn(&str) -> String) -> String {
    trigger.split('+').map(part).collect::<Vec<_>>().join(" + ")
}

fn pretty_modifier(part: &str) -> Option<&'static str> {
    Some(match part {
        "super" => "Super",
        "ctrl" => "Ctrl",
        "alt" => "Alt",
        "shift" => "Shift",
        "mod3" => "Mod3",
        _ => return None,
    })
}

fn pretty_key_part(part: &str) -> String {
    if let Some(m) = pretty_modifier(part) {
        return m.to_owned();
    }
    let mut chars = part.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn pretty_mouse_part(part: &str) -> String {
    match part {
        "left" => "Left button".to_owned(),
        "right" => "Right button".to_owned(),
        "middle" => "Middle button".to_owned(),
        "wheel-scroll" => "Wheel".to_owned(),
        "wheel-up" => "Wheel up".to_owned(),
        "wheel-down" => "Wheel down".to_owned(),
        "trackpad-scroll" => "Two-finger scroll".to_owned(),
        other => pretty_modifier(other).map_or_else(|| other.to_owned(), str::to_owned),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Row<'a> {
    Header(Group),
    Entry(&'a Entry),
}

/// The entries matching every whitespace-separated term of `filter` (case
/// insensitive, against keys, wording, raw action and section), each section
/// preceded by a header.
pub fn visible_rows<'a>(entries: &'a [Entry], filter: &str) -> Vec<Row<'a>> {
    let terms: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let mut rows = Vec::new();
    let mut current = None;
    for entry in entries {
        let hay = entry.haystack();
        if !terms.iter().all(|t| hay.contains(t.as_str())) {
            continue;
        }
        if current != Some(entry.group) {
            current = Some(entry.group);
            rows.push(Row::Header(entry.group));
        }
        rows.push(Row::Entry(entry));
    }
    rows
}

#[derive(Debug)]
pub struct App {
    pub mod_key: String,
    pub entries: Vec<Entry>,
    pub filter: String,
    pub scroll: usize,
    /// Body height from the last frame, for page-wise scrolling.
    pub viewport: usize,
}

impl App {
    pub fn new(bindings: &Bindings) -> Self {
        Self {
            mod_key: bindings.mod_key.clone(),
            entries: entries(bindings),
            filter: String::new(),
            scroll: 0,
            viewport: 10,
        }
    }

    pub fn rows(&self) -> Vec<Row<'_>> {
        visible_rows(&self.entries, &self.filter)
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta);
    }

    pub fn edit_filter(&mut self, edit: impl FnOnce(&mut String)) {
        edit(&mut self.filter);
        self.scroll = 0;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"{"Ok":{"Bindings":{"mod_key":"super",
      "keys":[
        {"trigger":"super+Return","action":"exec-terminal"},
        {"trigger":"super+q","action":"close-window"},
        {"trigger":"super+Up","action":"focus-nearest up"},
        {"trigger":"super+equal","action":"zoom-in"},
        {"trigger":"super+w","action":"spawn kitty --class dropdown"},
        {"trigger":"super+Foo","action":"brand-new-action 3"}],
      "taps":[{"trigger":"alt+shift","action":"switch-layout next"}],
      "mouse":[
        {"trigger":"alt+left","context":"on-window","action":"move-window"},
        {"trigger":"super+wheel-scroll","action":"zoom"}]}}}"#;

    pub(crate) fn sample() -> Vec<Entry> {
        entries(&parse_reply(SAMPLE).unwrap())
    }

    #[test]
    fn compositor_errors_pass_through() {
        assert_eq!(
            parse_reply(r#"{"Err":"no such thing"}"#).unwrap_err(),
            "no such thing"
        );
    }

    #[test]
    fn garbage_is_reported_not_panicked_on() {
        assert!(
            parse_reply("not json")
                .unwrap_err()
                .contains("unexpected reply")
        );
    }

    #[test]
    fn keys_read_like_keycaps() {
        let e = sample();
        let term = e.iter().find(|e| e.raw == "exec-terminal").unwrap();
        assert_eq!(term.keys, "Super + Return");
        assert_eq!(term.what, "Open a terminal");
        let close = e.iter().find(|e| e.raw == "close-window").unwrap();
        assert_eq!(close.keys, "Super + Q");
    }

    #[test]
    fn taps_and_mouse_get_their_own_sections_and_wording() {
        let e = sample();
        let tap = e.iter().find(|e| e.group == Group::Taps).unwrap();
        assert_eq!(tap.keys, "Alt + Shift (tap)");
        let drag = e.iter().find(|e| e.raw == "move-window").unwrap();
        assert_eq!(drag.group, Group::Mouse);
        assert_eq!(drag.keys, "Alt + Left button");
        assert_eq!(drag.what, "Move window (on a window)");
    }

    #[test]
    fn unknown_actions_fall_back_to_the_raw_string() {
        let e = sample();
        let new = e.iter().find(|e| e.keys == "Super + Foo").unwrap();
        assert_eq!(
            (new.group, new.what.as_str()),
            (Group::Other, "brand-new-action 3")
        );
    }

    #[test]
    fn rows_are_grouped_in_section_order_with_headers() {
        let e = sample();
        let headers: Vec<_> = visible_rows(&e, "")
            .into_iter()
            .filter_map(|r| match r {
                Row::Header(g) => Some(g),
                Row::Entry(_) => None,
            })
            .collect();
        assert_eq!(
            headers,
            [
                Group::Apps,
                Group::Windows,
                Group::Navigation,
                Group::Zoom,
                Group::Mouse,
                Group::Taps,
                Group::Other
            ]
        );
    }

    #[test]
    fn filter_terms_are_anded_and_case_insensitive() {
        let e = sample();
        let hits = |q: &str| {
            visible_rows(&e, q)
                .into_iter()
                .filter(|r| matches!(r, Row::Entry(_)))
                .count()
        };
        assert_eq!(hits("ZOOM"), 2);
        assert_eq!(hits("zoom super"), 2);
        assert_eq!(hits("zoom wheel"), 1);
        assert_eq!(hits("nothing like this"), 0);
        assert!(visible_rows(&e, "nothing like this").is_empty());
    }

    #[test]
    fn commands_lose_their_paths_and_script_interpreter() {
        assert_eq!(
            summarize_command("bash /home/dex/.config/driftwm/capture.sh shot-region"),
            "capture.sh shot-region"
        );
        assert_eq!(summarize_command("bash -c 'pkill x'"), "bash -c 'pkill x'");
        assert_eq!(summarize_command("playerctl next"), "playerctl next");
    }

    #[test]
    fn keys_bound_to_the_same_action_share_a_row() {
        let json = r#"{"Ok":{"Bindings":{"mod_key":"super","taps":[],"mouse":[],"keys":[
          {"trigger":"super+Left","action":"focus-nearest left"},
          {"trigger":"super+h","action":"focus-nearest left"},
          {"trigger":"super+Right","action":"focus-nearest right"}]}}}"#;
        let e = entries(&parse_reply(json).unwrap());
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].keys, "Super + H / Super + Left");
    }

    #[test]
    fn same_wording_in_different_sections_stays_apart() {
        let e = sample();
        assert_eq!(e.iter().filter(|e| e.what.starts_with("Zoom")).count(), 2);
    }

    #[test]
    fn filter_matches_the_raw_action_and_section_name() {
        let e = sample();
        assert!(!visible_rows(&e, "focus-nearest").is_empty());
        assert!(!visible_rows(&e, "modifier taps").is_empty());
    }
}
