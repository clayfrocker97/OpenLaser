// SPDX-License-Identifier: GPL-3.0-or-later

//! Command-line parsing without process exits or machine side effects.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub(crate) const HELP: &str = "openlaser [--config machine.toml] [--data DIR] [--listen ADDR] [--lan] [--simulate] [--no-browser] [--shutdown]\nopenlaser --version";

#[allow(
    clippy::struct_excessive_bools,
    reason = "Independent command-line switches can be combined"
)]
pub(crate) struct Arguments {
    pub config: PathBuf,
    pub config_explicit: bool,
    pub data: Option<PathBuf>,
    pub listen: Option<String>,
    pub simulate: bool,
    pub no_browser: bool,
    pub lan: bool,
    pub shutdown: bool,
}

pub(crate) enum CommandLine {
    Help,
    Version,
    Run(Arguments),
}

pub(crate) fn parse(words: impl IntoIterator<Item = OsString>) -> Result<CommandLine, String> {
    let mut arguments = Arguments {
        config: super::desktop::default_config(),
        config_explicit: false,
        data: None,
        listen: None,
        simulate: false,
        no_browser: false,
        lan: false,
        shutdown: false,
    };
    let mut words = words.into_iter();
    while let Some(word) = words.next() {
        if matches!(word.to_str(), Some("--help" | "-h")) {
            return Ok(CommandLine::Help);
        }
        if matches!(word.to_str(), Some("--version" | "-V")) {
            return Ok(CommandLine::Version);
        }
        arguments.option(&word, &mut words)?;
    }
    Ok(CommandLine::Run(arguments))
}

impl Arguments {
    fn option(
        &mut self,
        word: &OsStr,
        words: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), String> {
        match word.to_str() {
            Some("--config") => {
                self.config = value(words, "--config needs a path")?.into();
                self.config_explicit = true;
            }
            Some("--data") => {
                self.data = Some(value(words, "--data needs a directory")?.into());
            }
            Some("--listen") => {
                self.listen = Some(
                    value(words, "--listen needs an address")?
                        .into_string()
                        .map_err(|_| "--listen needs an IPv4 or IPv6 address and port")?,
                );
            }
            Some("--simulate") => self.simulate = true,
            Some("--no-browser") => self.no_browser = true,
            Some("--lan") => self.lan = true,
            Some("--shutdown") => self.shutdown = true,
            _ => return Err(format!("unknown argument {}", word.to_string_lossy())),
        }
        Ok(())
    }
}

fn value(words: &mut impl Iterator<Item = OsString>, missing: &str) -> Result<OsString, String> {
    words.next().ok_or_else(|| missing.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(words: &[&str]) -> Arguments {
        let Ok(CommandLine::Run(arguments)) = parse(words.iter().map(OsString::from)) else {
            panic!("valid arguments")
        };
        arguments
    }

    #[test]
    fn defaults_and_explicit_paths_remain_distinct() {
        let defaults = arguments(&[]);
        assert!(!defaults.config_explicit && !defaults.simulate && !defaults.no_browser);
        let selected = arguments(&[
            "--config",
            "my machine.toml",
            "--data",
            "my parts",
            "--listen",
            "127.0.0.1:9000",
            "--simulate",
            "--no-browser",
        ]);
        assert!(selected.config_explicit && selected.simulate && selected.no_browser);
        assert_eq!(selected.config, PathBuf::from("my machine.toml"));
        assert_eq!(selected.data, Some(PathBuf::from("my parts")));
        assert_eq!(selected.listen.as_deref(), Some("127.0.0.1:9000"));
    }

    #[test]
    fn help_missing_values_and_unknown_switches_are_decoded_without_exiting() {
        for flag in ["--help", "-h"] {
            assert!(matches!(parse([OsString::from(flag)]), Ok(CommandLine::Help)));
        }
        for flag in ["--version", "-V"] {
            assert!(matches!(parse([OsString::from(flag)]), Ok(CommandLine::Version)));
        }
        for flag in ["--config", "--data", "--listen", "--unknown"] {
            assert!(parse([OsString::from(flag)]).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_file_names_are_preserved() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'd', 0xff]);
        let Ok(CommandLine::Run(args)) = parse([OsString::from("--data"), path.clone()]) else {
            panic!("valid path")
        };
        assert_eq!(args.data, Some(PathBuf::from(path.clone())));
        assert!(parse([OsString::from("--listen"), path]).is_err());
    }
}
