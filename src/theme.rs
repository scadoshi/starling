//! Color themes: the terminal's own colors, or one of orangui's palettes in dark or light.
//!
//! The palettes themselves are generated into `palettes.rs`. A theme is stored in `config.json` as `terminal` or `<slug>-<mode>`, the same names orangui's CSS classes use.

use std::fmt;

use ratatui::style::{Color, Style};

use crate::palettes::{FAMILIES, GROUPS};

/// The stored name of the terminal's own colors.
const TERMINAL: &str = "terminal";

/// A 24-bit color: red, green, blue.
pub(crate) type Rgb = [u8; 3];

/// One mode of an orangui theme, by the role each color plays in Starling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Colors {
    pub(crate) background: Rgb,
    pub(crate) text: Rgb,
    /// Hints, timestamps and placeholders.
    pub(crate) muted: Rgb,
    pub(crate) border: Rgb,
    /// The selection and other people's names.
    pub(crate) accent: Rgb,
    /// The user's own messages and the focused box.
    pub(crate) success: Rgb,
    pub(crate) error: Rgb,
    /// Attachments and images.
    pub(crate) warning: Rgb,
}

/// An orangui theme: its slug, its display name, and both modes.
#[derive(Debug)]
pub(crate) struct Family {
    pub(crate) slug: &'static str,
    pub(crate) name: &'static str,
    pub(crate) dark: Colors,
    pub(crate) light: Colors,
}

/// A section of the theme picker.
#[derive(Debug)]
pub(crate) struct Group {
    pub(crate) label: &'static str,
    pub(crate) slugs: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Dark,
    Light,
}

impl Mode {
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dark => "dark",
            Self::Light => "light",
        })
    }
}

/// A chosen theme. `family` is an orangui slug, or `None` for the terminal's own colors, which ignore `mode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Theme {
    pub(crate) family: Option<&'static str>,
    pub(crate) mode: Mode,
}

impl Theme {
    /// Reads a stored name: `terminal`, or a known slug followed by `-dark` or `-light`.
    pub(crate) fn parse(stored: &str) -> Option<Self> {
        if stored == TERMINAL {
            return Some(Self::default());
        }
        let (slug, mode) = stored.rsplit_once('-')?;
        let mode = match mode {
            "dark" => Mode::Dark,
            "light" => Mode::Light,
            _ => return None,
        };
        let family = family(slug)?;
        Some(Self {
            family: Some(family.slug),
            mode,
        })
    }

    /// The name stored in `config.json`, which [`Theme::parse`] reads back.
    pub(crate) fn key(self) -> String {
        match self.family {
            None => TERMINAL.to_owned(),
            Some(slug) => format!("{slug}-{}", self.mode),
        }
    }

    pub(crate) fn palette(self) -> Palette {
        let Some(family) = self.family.and_then(family) else {
            return Palette::terminal();
        };
        Palette::from_colors(match self.mode {
            Mode::Dark => &family.dark,
            Mode::Light => &family.light,
        })
    }
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.family.and_then(family) {
            None => f.write_str("Terminal"),
            Some(family) => write!(f, "{} {}", family.name, self.mode),
        }
    }
}

fn family(slug: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.slug == slug)
}

/// A theme's display name by slug, or the slug itself when it is unknown.
pub(crate) fn name(slug: &'static str) -> &'static str {
    family(slug).map_or(slug, |f| f.name)
}

/// The styles every part of the UI draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// Text and background for every cell.
    pub(crate) base: Style,
    pub(crate) muted: Style,
    pub(crate) border: Style,
    pub(crate) accent: Style,
    /// The user's own messages and the focused box.
    pub(crate) me: Style,
    pub(crate) error: Style,
    pub(crate) attachment: Style,
}

impl Palette {
    /// The terminal's own colors, by their ANSI names.
    fn terminal() -> Self {
        Self {
            base: Style::new(),
            muted: Style::new().dim(),
            border: Style::new(),
            accent: Style::new().cyan(),
            me: Style::new().green(),
            error: Style::new().red(),
            attachment: Style::new().yellow(),
        }
    }

    fn from_colors(colors: &Colors) -> Self {
        let fg = |[r, g, b]: Rgb| Style::new().fg(Color::Rgb(r, g, b));
        let [r, g, b] = colors.background;
        Self {
            base: fg(colors.text).bg(Color::Rgb(r, g, b)),
            muted: fg(colors.muted),
            border: fg(colors.border),
            accent: fg(colors.accent),
            me: fg(colors.success),
            error: fg(colors.error),
            attachment: fg(colors.warning),
        }
    }
}

/// A row of the theme picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    /// A section label, which the selection skips.
    Header(&'static str),
    /// A theme family, or `None` for the terminal's colors.
    Choice(Option<&'static str>),
}

/// The picker's rows: the terminal's colors first, then each group under its label.
pub(crate) fn rows() -> Vec<Row> {
    let mut rows = vec![Row::Choice(None)];
    for group in GROUPS {
        rows.push(Row::Header(group.label));
        rows.extend(group.slugs.iter().map(|&slug| Row::Choice(Some(slug))));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_grouped_theme_has_a_palette_and_every_palette_a_group() {
        let grouped: Vec<&str> = GROUPS
            .iter()
            .flat_map(|g| g.slugs.iter().copied())
            .collect();
        assert_eq!(grouped.len(), FAMILIES.len());
        for slug in grouped {
            assert!(family(slug).is_some(), "{slug} has no palette");
        }
    }

    #[test]
    fn matches_orangui_at_generation_time() {
        assert_eq!(FAMILIES.len(), 50, "orangui ships 50 themes");
        assert_eq!(name("rose-pine"), "Rosé Pine");
        assert_eq!(name("monokai-pro-spectrum"), "Monokai Pro Spectrum");
        let gruvbox = family("gruvbox").unwrap();
        assert_eq!(gruvbox.dark.background, [0x28, 0x28, 0x28]);
        assert_eq!(gruvbox.light.background, [0xfb, 0xf1, 0xc7]);
    }

    #[test]
    fn stored_names_round_trip_for_every_theme() {
        let mut all = vec![Theme::default()];
        for f in FAMILIES {
            for mode in [Mode::Dark, Mode::Light] {
                all.push(Theme {
                    family: Some(f.slug),
                    mode,
                });
            }
        }
        for theme in all {
            assert_eq!(Theme::parse(&theme.key()), Some(theme), "{}", theme.key());
        }
    }

    #[test]
    fn parse_refuses_unknown_names() {
        for stored in ["", "gruvbox", "gruvbox-dim", "nope-dark", "terminal-dark"] {
            assert_eq!(Theme::parse(stored), None, "{stored}");
        }
    }

    #[test]
    fn slugs_with_hyphens_parse_by_the_last_one() {
        let theme = Theme::parse("monokai-pro-ristretto-light").unwrap();
        assert_eq!(theme.family, Some("monokai-pro-ristretto"));
        assert_eq!(theme.mode, Mode::Light);
        assert_eq!(theme.to_string(), "Monokai Pro Ristretto light");
    }

    #[test]
    fn orangui_palettes_set_a_background_and_the_terminal_does_not() {
        assert_eq!(Theme::default().palette().base.bg, None);
        let gruvbox = Theme::parse("gruvbox-dark").unwrap().palette();
        assert_eq!(gruvbox.base.bg, Some(Color::Rgb(0x28, 0x28, 0x28)));
    }

    #[test]
    fn rows_start_with_the_terminal_and_label_every_group() {
        let rows = rows();
        assert_eq!(rows.first(), Some(&Row::Choice(None)));
        let headers = rows.iter().filter(|r| matches!(r, Row::Header(_))).count();
        assert_eq!(headers, GROUPS.len());
        assert_eq!(rows.len(), 1 + GROUPS.len() + FAMILIES.len());
    }
}
