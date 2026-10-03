//! The theme registry (#166): names, the role set each name has per colour
//! mode, and the promise that a second theme leaves the first as it was.

pub mod common;

use common::contrast::{contrast, luminance, rgb};
use dev_cleaner::tui::palette::{Mode, Ramp, Theme, ThemeName};
use ratatui::style::{Color, Modifier, Style};

const BLACK: (u8, u8, u8) = (0, 0, 0);

#[test]
fn the_registry_lists_every_theme_once_with_a_line_that_says_what_it_is() {
    let ids: Vec<&str> = ThemeName::ALL.iter().map(|e| e.id).collect();
    assert_eq!(ids, ["neon", "matrix"]);
    for entry in ThemeName::ALL {
        assert!(!entry.about.is_empty(), "{} has no description", entry.id);
        assert!(!entry.about.contains('\n'), "{} is one line", entry.id);
        assert_eq!(ThemeName::parse(entry.id), Some(entry.name));
        assert_eq!(entry.name.id(), entry.id);
    }
    assert_eq!(ThemeName::parse("bogus"), None);
    assert_eq!(ThemeName::parse(""), None);
    assert_eq!(ThemeName::parse("Neon"), None, "names are exact");
}

#[test]
fn next_walks_the_registry_and_comes_back_to_where_it_started() {
    let mut at = ThemeName::Neon;
    let mut seen = Vec::new();
    for _ in 0..ThemeName::ALL.len() {
        at = at.next();
        seen.push(at);
    }
    assert_eq!(seen, [ThemeName::Matrix, ThemeName::Neon]);
}

#[test]
fn neon_by_name_is_the_neon_that_was_there_before() {
    // Debug is every field, and the fields are every style, glyph and flag.
    for (mode, old) in [
        (Mode::Truecolor, Theme::neon()),
        (Mode::Ansi, Theme::ansi()),
        (Mode::Mono, Theme::mono()),
    ] {
        assert_eq!(
            format!("{:?}", Theme::named(ThemeName::Neon, mode)),
            format!("{old:?}")
        );
    }
}

#[test]
fn every_name_has_a_theme_in_every_colour_mode_and_says_which_it_is() {
    for entry in ThemeName::ALL {
        for mode in [Mode::Truecolor, Mode::Ansi, Mode::Mono] {
            let theme = Theme::named(entry.name, mode);
            assert_eq!(theme.mode(), mode);
            assert_eq!(theme.name(), entry.name);
        }
    }
}

#[test]
fn no_colour_is_no_colour_for_every_name() {
    for entry in ThemeName::ALL {
        let t = Theme::named(entry.name, Mode::Mono);
        let styles = [
            t.ground,
            t.text,
            t.muted,
            t.head,
            t.accent,
            t.key,
            t.safe,
            t.blocked,
            t.danger,
            t.violet,
            t.selected,
            t.warning_band,
            t.verdict_safe,
            t.verdict_blocked,
            t.size(0),
            t.size(u64::MAX),
            t.ramp(Ramp::Measure, 3, 9),
            t.ramp(Ramp::Danger, 3, 9),
            t.brand(0.5),
            t.logo_can,
            t.logo_mark,
        ];
        for s in styles {
            assert_eq!((s.fg, s.bg), (None, None), "{}: {s:?}", entry.id);
        }
        assert_eq!(t.progress_line("12 · 34"), "12 · 34");
    }
}

#[test]
fn the_256_colour_look_names_ansi_colours_only() {
    for entry in ThemeName::ALL {
        let t = Theme::named(entry.name, Mode::Ansi);
        let styles = [
            t.ground,
            t.text,
            t.muted,
            t.head,
            t.accent,
            t.key,
            t.safe,
            t.blocked,
            t.danger,
            t.violet,
            t.selected,
            t.warning_band,
            t.size(0),
            t.size(u64::MAX),
            t.ramp(Ramp::Measure, 0, 9),
            t.ramp(Ramp::Measure, 8, 9),
            t.brand(0.0),
            t.brand(1.0),
            t.logo_can,
            t.logo_mark,
        ];
        for s in styles {
            for colour in [s.fg, s.bg].into_iter().flatten() {
                assert!(
                    !matches!(colour, Color::Rgb(..) | Color::Indexed(_)),
                    "{}: {s:?}",
                    entry.id
                );
            }
        }
        assert_eq!(t.ground, Style::new(), "the profile keeps its background");
    }
}

fn matrix() -> Theme {
    Theme::named(ThemeName::Matrix, Mode::Truecolor)
}

fn fg(s: Style) -> (u8, u8, u8) {
    rgb(s.fg)
}

#[test]
fn matrix_is_phosphor_on_true_black() {
    let t = matrix();
    assert_eq!(rgb(t.ground.bg), BLACK);
    assert_eq!(fg(t.text), (0x33, 0xff, 0x66));
    assert_eq!(fg(t.muted), (0x1f, 0x9a, 0x3f));
    assert_eq!(fg(t.head), (0xcc, 0xff, 0xdd));
    assert!(t.head.add_modifier.contains(Modifier::BOLD));
    assert_eq!(fg(t.accent), (0x00, 0xff, 0x41));
    // The DOS highlight bar: black on bright green.
    assert_eq!(fg(t.selected), BLACK);
    assert_eq!(rgb(t.selected.bg), (0x00, 0xff, 0x41));
    assert_eq!(fg(t.blocked), (0xff, 0xb0, 0x00));
    assert_eq!(fg(t.danger), (0xff, 0x40, 0x40));
}

#[test]
fn the_logo_is_two_greens_the_can_bright_and_the_mark_white_green() {
    let t = matrix();
    assert_eq!(fg(t.logo_can), (0x00, 0xff, 0x41));
    assert_eq!(fg(t.logo_mark), (0xcc, 0xff, 0xdd));
    // Neon's logo is what it was: the head and the accent.
    let n = Theme::neon();
    assert_eq!(n.logo_can.fg, n.head.fg);
    assert_eq!(n.logo_mark.fg, n.accent.fg);
}

#[test]
fn every_matrix_text_role_reads_on_black() {
    let t = matrix();
    for (role, style) in [
        ("text", t.text),
        ("muted", t.muted),
        ("head", t.head),
        ("accent", t.accent),
        ("key", t.key),
        ("safe", t.safe),
        ("blocked", t.blocked),
        ("danger", t.danger),
        ("violet", t.violet),
        ("verdict_safe", t.verdict_safe),
        ("verdict_blocked", t.verdict_blocked),
    ] {
        let ratio = contrast(fg(style), BLACK);
        assert!(ratio >= 4.5, "{role} is {ratio:.2}:1 on black");
    }
    // The inverse bars: the ink on the fill.
    for (role, style) in [("selected", t.selected), ("warning band", t.warning_band)] {
        let ratio = contrast(rgb(style.fg), rgb(style.bg));
        assert!(ratio >= 4.5, "{role} is {ratio:.2}:1");
    }
}

#[test]
fn every_matrix_ink_shape_reads_on_black() {
    let t = matrix();
    let mut inks = vec![t.logo_can, t.logo_mark];
    for i in 0..9 {
        inks.push(t.ramp(Ramp::Measure, i, 9));
        inks.push(t.ramp(Ramp::Danger, i, 9));
        inks.push(t.brand(i as f32 / 8.0));
    }
    for bytes in [0, 200 << 20, 2 << 30] {
        inks.push(t.size(bytes));
    }
    for ink in inks {
        let ratio = contrast(fg(ink), BLACK);
        assert!(ratio >= 3.0, "{ink:?} is {ratio:.2}:1 on black");
    }
}

#[test]
fn matrix_meaning_does_not_live_in_green_alone() {
    let t = matrix();
    let green = |c: (u8, u8, u8)| c.1 > c.0 && c.1 > c.2;
    // Held back and danger are not shades of ok: neither is a green.
    assert!(!green(fg(t.blocked)), "blocked is green");
    assert!(!green(fg(t.danger)), "danger is green");
    assert_ne!(fg(t.blocked), fg(t.danger));
    assert_ne!(fg(t.blocked), fg(t.safe));
    assert_ne!(fg(t.danger), fg(t.safe));
    // The same in 16 colours.
    let a = Theme::named(ThemeName::Matrix, Mode::Ansi);
    assert_eq!(a.blocked.fg, Some(Color::Yellow));
    assert_eq!(a.danger.fg, Some(Color::Red));
    assert!(matches!(a.safe.fg, Some(Color::Green | Color::LightGreen)));
}

#[test]
fn the_matrix_size_ramp_runs_dim_to_bright_so_the_largest_is_the_loudest() {
    let t = matrix();
    let lum = |s: Style| luminance(fg(s));
    let ramp: Vec<f64> = (0..9).map(|i| lum(t.ramp(Ramp::Measure, i, 9))).collect();
    assert!(ramp.windows(2).all(|w| w[0] <= w[1]), "{ramp:?}");
    assert!(ramp[8] > ramp[0] * 2.0, "{ramp:?}");
    let sizes = [t.size(1), t.size(200 << 20), t.size(2 << 30)];
    let lums: Vec<f64> = sizes.iter().map(|s| lum(*s)).collect();
    assert!(lums.windows(2).all(|w| w[0] <= w[1]), "{lums:?}");
    assert!(
        sizes[2].add_modifier.contains(Modifier::BOLD),
        "the biggest is bold as well as bright"
    );
}

#[test]
fn the_wordmark_runs_dark_green_to_white_green() {
    let t = matrix();
    let (first, last) = (t.brand(0.0), t.brand(1.0));
    assert!(luminance(fg(first)) < luminance(fg(last)));
    assert_eq!(fg(first), (0x1f, 0x9a, 0x3f));
    assert_eq!(fg(last), (0xcc, 0xff, 0xdd));
    assert!(first.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn matrix_draws_bars_in_shaded_blocks_and_ascii_where_the_font_is_not_trusted() {
    let m = matrix().cells();
    assert_eq!((m.full, m.mark, m.empty), ('█', '▒', '░'));
    assert!(!m.bracketed);
    let ascii = matrix().for_term(Some("linux")).cells();
    assert_eq!((ascii.full, ascii.empty), ('#', '-'));
    assert!(ascii.bracketed);
}

#[test]
fn matrix_rules_are_double_lines_and_neon_keeps_its_single_ones() {
    assert_eq!(matrix().rule(), '═');
    assert_eq!(Theme::neon().rule(), '─');
    assert_eq!(matrix().rule_line(6), "╞════╡");
    assert_eq!(Theme::neon().rule_line(6), "──────");
}

#[test]
fn matrix_key_caps_are_bracketed_and_neon_has_none() {
    assert_eq!(matrix().caps(), ("[", "]"));
    assert_eq!(Theme::neon().caps(), ("", ""));
    // Under NO_COLOR a cap is still a cap: a shape, not a colour.
    assert_eq!(
        Theme::named(ThemeName::Matrix, Mode::Mono).caps(),
        ("[", "]")
    );
}

#[test]
fn the_stepper_markers_are_bracketed_in_matrix_and_ascii_safe_on_a_console() {
    assert_eq!(matrix().steps(), ["[✓] ", "[●] ", "[ ] "]);
    assert_eq!(Theme::neon().steps(), ["✓ ", "● ", "○ "]);
    assert_eq!(
        matrix().for_term(Some("linux")).steps(),
        ["[x] ", "[*] ", "[ ] "]
    );
}

#[test]
fn a_live_switch_keeps_the_colour_mode_the_icons_and_the_motion_setting() {
    let old = Theme::named(ThemeName::Neon, Mode::Ansi)
        .for_term(Some("linux"))
        .with_motion(false);
    let new = old.switched(ThemeName::Matrix);
    assert_eq!(new.name(), ThemeName::Matrix);
    assert_eq!(new.mode(), Mode::Ansi);
    assert!(!new.braille(), "the console's icon set survives the switch");
    assert_eq!(new.cells().full, '#');
    assert!(!new.rain());
}

#[test]
fn the_rain_is_a_matrix_thing_that_colour_and_reduced_motion_both_switch_off() {
    for (theme, rain) in [
        (Theme::named(ThemeName::Matrix, Mode::Truecolor), true),
        (Theme::named(ThemeName::Matrix, Mode::Ansi), true),
        (Theme::named(ThemeName::Matrix, Mode::Mono), false),
        (Theme::named(ThemeName::Neon, Mode::Truecolor), false),
        (Theme::named(ThemeName::Neon, Mode::Ansi), false),
        (matrix().with_motion(false), false),
    ] {
        assert_eq!(theme.rain(), rain, "{theme:?}");
    }
    assert!(matrix().motion_from(None).rain());
    assert!(!matrix().motion_from(Some("1")).rain());
    assert!(matrix().motion_from(Some("0")).rain());
}

#[test]
fn the_cli_line_is_painted_in_the_theme_and_neon_keeps_its_bytes() {
    assert_eq!(
        Theme::neon().progress_line("1 · 2"),
        "\x1b[38;2;0;229;255m1 \x1b[38;2;180;140;255m·\x1b[38;2;0;229;255m 2\x1b[0m"
    );
    assert_eq!(
        Theme::ansi().progress_line("1 · 2"),
        "\x1b[36m1 \x1b[35m·\x1b[36m 2\x1b[0m"
    );
    let line = matrix().progress_line("1 · 2");
    assert!(line.starts_with("\x1b[38;2;0;255;65m"), "{line:?}");
    assert!(line.ends_with("\x1b[0m"));
    let green = Theme::named(ThemeName::Matrix, Mode::Ansi).progress_line("1 · 2");
    assert!(green.starts_with("\x1b[92m"), "{green:?}");
}

#[test]
fn paint_wraps_text_in_a_role_and_adds_nothing_without_colour() {
    let t = matrix();
    assert_eq!(
        t.paint(t.blocked, "held"),
        "\x1b[38;2;255;176;0mheld\x1b[0m"
    );
    assert_eq!(
        t.paint(t.danger, "held"),
        "\x1b[1;38;2;255;64;64mheld\x1b[0m"
    );
    let plain = Theme::named(ThemeName::Matrix, Mode::Mono);
    assert_eq!(plain.paint(plain.danger, "held"), "held");
}
