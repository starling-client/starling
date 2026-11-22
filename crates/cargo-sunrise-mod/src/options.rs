//! A command's options, read once by the code that uses them: whatever the command does not read
//! is refused by name in [`Options::finish`], so a mistyped option or a stray word is never
//! silently ignored. Each argument has one role, fixed before anything is read: the argument after
//! an option that takes a value is that value, whatever it looks like, and never an option itself.

/// The arguments of one command (after its name), with what has been read from them so far.
pub struct Options<'a> {
    command: &'a str,
    args: &'a [String],
    read: Vec<bool>,
    /// The arguments that are the value of the option before them.
    is_value: Vec<bool>,
    /// Where a bare `--` begins what the command hands on untouched (`run`'s game arguments).
    end: usize,
    /// Whether the command asked for what follows that `--`.
    rest_read: bool,
    /// The usage a refusal shows, where it is not the help table's.
    usage: Option<&'a str>,
}

impl<'a> Options<'a> {
    /// `command` is the name the refusal uses (`registry list`); its first word names the usage.
    /// `valued` names the options that take a value, which [`Options::value`] and
    /// [`Options::values`] then read.
    pub fn new(command: &'a str, args: &'a [String], valued: &[&str]) -> Self {
        let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
        let mut is_value = vec![false; args.len()];
        let mut i = 0;
        while i + 1 < end {
            if valued.contains(&args[i].as_str()) {
                is_value[i + 1] = true;
                i += 1;
            }
            i += 1;
        }
        Self {
            command,
            args,
            read: vec![false; args.len()],
            is_value,
            end,
            rest_read: false,
            usage: None,
        }
    }

    /// Shows `usage` in a refusal, for a command whose forms the help table only names.
    pub fn usage(mut self, usage: &'a str) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Whether the option `name` stands among the arguments.
    pub fn switch(&mut self, name: &str) -> bool {
        self.take(name).count() > 0
    }

    /// The value after `name` (the first one, if it is given again); it may begin with a dash.
    /// Refuses `name` as the last argument, which has no value.
    pub fn value(&mut self, name: &str) -> Result<Option<&'a str>, String> {
        Ok(self.values(name)?.first().copied())
    }

    /// The value after each `name`, in order.
    pub fn values(&mut self, name: &str) -> Result<Vec<&'a str>, String> {
        let mut out = Vec::new();
        for i in self.take(name).collect::<Vec<_>>() {
            if i + 1 >= self.end {
                return Err(self.refusal(&format!("{name} needs a value")));
            }
            self.read[i + 1] = true;
            out.push(self.args[i + 1].as_str());
        }
        Ok(out)
    }

    /// The arguments that are neither options nor their values, in order.
    pub fn positional(&mut self) -> Vec<&'a str> {
        let mut out = Vec::new();
        for i in 0..self.end {
            if !self.read[i] && !self.is_value[i] && !is_option(&self.args[i]) {
                self.read[i] = true;
                out.push(self.args[i].as_str());
            }
        }
        out
    }

    /// The one argument that is not an option, if it is given; a second one is refused by name.
    /// Read the options first: a mistyped option is named before the value it left behind.
    pub fn at_most_one(&mut self) -> Result<Option<&'a str>, String> {
        let mistyped =
            (0..self.end).find(|&i| !self.read[i] && !self.is_value[i] && is_option(&self.args[i]));
        match self.positional()[..] {
            [] => Ok(None),
            [one] => Ok(Some(one)),
            [_, second, ..] => {
                let stray = mistyped.map_or(second, |i| self.args[i].as_str());
                Err(self.refusal(&format!("{} does not take {stray}", self.command)))
            }
        }
    }

    /// What follows a bare `--`.
    pub fn rest(&mut self) -> &'a [String] {
        self.rest_read = true;
        self.args.get(self.end + 1..).unwrap_or_default()
    }

    /// Refuses the first argument nothing read, naming it and showing the command's usage: an
    /// option, a word the command has no place for, or a `--` with a tail it does not hand on.
    pub fn finish(self) -> Result<(), String> {
        let unread = (0..self.end)
            .find(|&i| !self.read[i])
            .or_else(|| (self.end < self.args.len() && !self.rest_read).then_some(self.end));
        match unread {
            Some(i) => {
                Err(self.refusal(&format!("{} does not take {}", self.command, self.args[i])))
            }
            None => Ok(()),
        }
    }

    fn take<'s>(&'s mut self, name: &'s str) -> impl Iterator<Item = usize> + 's {
        (0..self.end).filter(move |&i| {
            let hit = !self.is_value[i] && self.args[i] == name;
            self.read[i] |= hit;
            hit
        })
    }

    fn refusal(&self, what: &str) -> String {
        let usage = match self.usage {
            Some(usage) => usage.to_owned(),
            None => self
                .command
                .split(' ')
                .next()
                .and_then(crate::help::of)
                .unwrap_or_default(),
        };
        format!("{what}\n{}", usage.trim_end())
    }
}

fn is_option(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(line: &str) -> Vec<String> {
        line.split(' ')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn what_no_one_read_is_refused_by_name_with_the_usage() {
        let args = argv("--game g --typo extra");
        let mut options = Options::new("run", &args, &["--game", "--name", "--jars"]);
        assert_eq!(options.value("--game").unwrap(), Some("g"));
        let error = options.finish().unwrap_err();
        assert!(error.contains("--typo"), "{error}");
        assert!(error.contains("usage: cargo sunrise-mod run"), "{error}");
    }

    #[test]
    fn a_stray_word_a_second_file_and_an_unwanted_tail_are_refused_by_name() {
        // `update nightly` without `--sunrise` would otherwise repin to the current Nightly.
        let args = argv("nightly-2026-11-20");
        let error = Options::new("update", &args, &[]).finish().unwrap_err();
        assert!(error.contains("nightly-2026-11-20"), "{error}");

        let args = argv("a.smod b.smod");
        let error = Options::new("fits", &args, &["--build"])
            .at_most_one()
            .unwrap_err();
        assert!(
            error.contains("b.smod") && !error.contains("a.smod"),
            "{error}"
        );

        let args = argv("block -- break");
        let mut options = Options::new("hooks", &args, &[]);
        assert_eq!(options.positional(), ["block"]);
        let error = options.finish().unwrap_err();
        let refused = error.lines().next().unwrap().split(' ').next_back();
        assert_eq!(refused, Some("--"), "{error}");
    }

    #[test]
    fn an_options_value_is_never_also_an_option_whichever_is_read_first() {
        let args = argv("--signature --native --key --kind");
        let mut options = Options::new("check", &args, &["--signature", "--key", "--kind"]);
        assert!(!options.switch("--native"));
        assert_eq!(options.value("--kind").unwrap(), None);
        assert_eq!(options.value("--signature").unwrap(), Some("--native"));
        assert_eq!(options.value("--key").unwrap(), Some("--kind"));
        assert!(options.positional().is_empty());
        options.finish().unwrap();
    }

    #[test]
    fn values_may_begin_with_a_dash_and_stop_at_a_bare_double_dash() {
        let args = argv("--name -Garden pos --jars j -- --typo --game");
        let mut options = Options::new("run", &args, &["--game", "--name", "--jars"]);
        assert_eq!(options.value("--name").unwrap(), Some("-Garden"));
        assert_eq!(options.value("--jars").unwrap(), Some("j"));
        assert_eq!(options.positional(), ["pos"]);
        assert_eq!(options.rest(), ["--typo", "--game"]);
        options.finish().unwrap();
    }

    #[test]
    fn repeated_options_are_all_read_and_a_missing_value_is_refused() {
        let args = argv("--build a --build b --all");
        let mut options = Options::new("fits", &args, &["--build"]);
        assert_eq!(options.values("--build").unwrap(), ["a", "b"]);
        assert!(options.switch("--all"));
        options.finish().unwrap();

        let last = argv("--build");
        let error = Options::new("fits", &last, &["--build"])
            .value("--build")
            .unwrap_err();
        assert!(error.contains("--build"), "{error}");
    }
}
