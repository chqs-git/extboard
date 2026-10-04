use serde_json::{Map, Value};
use std::borrow::Cow;
use std::sync::LazyLock;

use crate::{Canvas, object_mut};

pub type Vars = Map<String, Value>;

const OPEN: &str = "{{";
const CLOSE: &str = "}}";

impl Canvas {
    pub fn vars(&self) -> &Vars {
        static EMPTY: LazyLock<Vars> = LazyLock::new(Map::new);
        self.extra
            .get("vars")
            .and_then(Value::as_object)
            .unwrap_or(&EMPTY)
    }

    pub fn set_var(&mut self, name: &str, value: &str) {
        object_mut(&mut self.extra, "vars")
            .insert(name.to_owned(), Value::String(value.to_owned()));
    }
}

pub fn interpolate<'a>(text: &'a str, vars: &Vars) -> Cow<'a, str> {
    if vars.is_empty() || !text.contains(OPEN) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(OPEN) {
        let after = &rest[open + OPEN.len()..];
        let Some(close) = after.find(CLOSE) else {
            break;
        };
        out.push_str(&rest[..open]);
        match lookup(vars, after[..close].trim()) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[open..open + OPEN.len() + close + CLOSE.len()]),
        }
        rest = &after[close + CLOSE.len()..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn lookup(vars: &Vars, path: &str) -> Option<String> {
    let mut keys = path.split('.');
    let mut value = vars.get(keys.next()?)?;
    for key in keys {
        value = value.get(key)?;
    }
    Some(match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars() -> Vars {
        json!({
            "Character-A": "Joao",
            "City": "Porto",
            "Monday": { "temperature": 21, "unit": "C" },
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn a_defined_var_is_substituted_and_an_undefined_one_reads_as_itself() {
        let got = interpolate("{{Character-A}} arrives in {{City}}, not {{Nope}}", &vars());
        assert_eq!(got, "Joao arrives in Porto, not {{Nope}}");
    }

    #[test]
    fn a_dotted_path_reaches_into_a_value() {
        let vars = vars();
        assert_eq!(interpolate("{{Monday.temperature}}", &vars), "21");
        assert_eq!(interpolate("{{ Monday.unit }}", &vars), "C");
        assert_eq!(interpolate("{{Monday.rain}}", &vars), "{{Monday.rain}}");
        assert_eq!(
            interpolate("{{Monday}}", &vars),
            r#"{"temperature":21,"unit":"C"}"#
        );
    }

    #[test]
    fn text_with_nothing_to_substitute_is_not_copied() {
        assert!(matches!(
            interpolate("plain prose", &vars()),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            interpolate("{{City}}", &Vars::new()),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn an_unclosed_brace_is_left_alone() {
        assert_eq!(interpolate("{{City", &vars()), "{{City");
        assert_eq!(interpolate("{{City}} then {{", &vars()), "Porto then {{");
    }

    #[test]
    fn the_vars_block_is_read_off_the_canvas() {
        let canvas: Canvas =
            serde_json::from_str(r#"{"vars":{"City":"Porto"},"nodes":[],"edges":[]}"#).unwrap();
        assert_eq!(interpolate("{{City}}", canvas.vars()), "Porto");
        assert!(Canvas::default().vars().is_empty());
    }

    #[test]
    fn a_var_can_be_set_on_a_canvas_with_no_vars_block() {
        let mut canvas = Canvas::default();
        canvas.set_var("Character-A", "Maria");
        assert_eq!(interpolate("{{Character-A}}", canvas.vars()), "Maria");

        canvas.set_var("Character-A", "Joao");
        assert_eq!(canvas.vars().len(), 1, "a set replaces rather than adds");
    }
}
