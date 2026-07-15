#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub top_level_usage: &'static [&'static str],
    pub usage: &'static [&'static str],
    pub notes: &'static [&'static str],
    pub options: &'static [&'static str],
}

mod catalog;

pub use self::catalog::COMMAND_SPECS;

pub fn command_spec(name: &str) -> Option<&'static CommandSpec> {
    COMMAND_SPECS.iter().find(|spec| spec.name == name)
}

pub fn help_text() -> String {
    let mut output = String::from(
        "runx\n\nUsage:\n  runx <command> [args]\n  runx --help\n  runx --version\n\nCommands:\n",
    );
    for spec in COMMAND_SPECS {
        let usage_lines = if spec.top_level_usage.is_empty() {
            spec.usage
        } else {
            spec.top_level_usage
        };
        for usage in usage_lines {
            output.push_str("  ");
            output.push_str(usage);
            output.push('\n');
        }
    }
    output
}

pub fn command_help_text(name: &str) -> Option<String> {
    let spec = command_spec(name)?;
    let mut output = format!("runx {}\n\nUsage:\n", spec.name);
    for usage in spec.usage {
        output.push_str("  ");
        output.push_str(usage);
        output.push('\n');
    }
    if !spec.notes.is_empty() {
        output.push('\n');
        for note in spec.notes {
            output.push_str(note);
            output.push('\n');
        }
    }
    if !spec.options.is_empty() {
        output.push_str("\nOptions:\n");
        for option in spec.options {
            output.push_str("  ");
            output.push_str(option);
            output.push('\n');
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{COMMAND_SPECS, command_help_text, help_text};

    #[test]
    fn command_names_are_unique_and_have_help() {
        let mut names = BTreeSet::new();
        for spec in COMMAND_SPECS {
            assert!(names.insert(spec.name), "duplicate command {}", spec.name);
            let help = command_help_text(spec.name);
            assert!(help.is_some(), "missing help for {}", spec.name);
        }
        assert_eq!(COMMAND_SPECS.len(), 23);
    }

    #[test]
    fn top_level_help_is_generated_from_command_usage() {
        let help = help_text();
        for spec in COMMAND_SPECS {
            let usage_lines = if spec.top_level_usage.is_empty() {
                spec.usage
            } else {
                spec.top_level_usage
            };
            for usage in usage_lines {
                assert!(help.lines().any(|line| line.trim() == *usage));
            }
        }
    }
}
