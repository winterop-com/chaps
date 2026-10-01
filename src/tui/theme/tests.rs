use super::*;

#[test]
fn the_default_palette_is_ansi_16_so_it_follows_the_terminal() {
    let theme = Theme::default();
    for colour in [
        theme.accent,
        theme.ok,
        theme.warn,
        theme.bad,
        theme.dim,
        theme.template,
        theme.selection_bg,
        theme.selection_fg,
    ] {
        assert!(
            !matches!(colour, Color::Rgb(..) | Color::Indexed(_)),
            "{colour:?} is not one of the 16 colours the terminal owns"
        );
    }
}

#[test]
fn monochrome_paints_nothing_and_still_tells_things_apart() {
    let theme = Theme::monochrome();
    assert!(theme.mono);
    for style in [
        theme.accent_style(),
        theme.ok_style(),
        theme.warn_style(),
        theme.bad_style(),
        theme.dim_style(),
        theme.template_style(),
        theme.column_style(),
        theme.chip_style(),
        theme.selection_style(),
        theme.status_style(AssessedStatus::Red),
        theme.version_style(VersionStatus::Yanked),
    ] {
        assert_eq!(style.fg, None, "{style:?} still carries a colour");
        assert_eq!(style.bg, None, "{style:?} still carries a colour");
        assert!(
            !style.add_modifier.is_empty(),
            "{style:?} says nothing at all without colour"
        );
    }
    assert!(
        theme
            .selection_style()
            .add_modifier
            .contains(Modifier::REVERSED),
        "the cursor is the one thing that must always be visible"
    );
}

#[test]
fn the_coloured_theme_keeps_the_states_apart() {
    let theme = Theme::default();
    assert_eq!(theme.status_style(AssessedStatus::Green).fg, Some(theme.ok));
    assert_eq!(
        theme.status_style(AssessedStatus::Orange).fg,
        Some(theme.warn),
        "orange has no 16-colour of its own, so it borrows yellow"
    );
    assert_eq!(theme.status_style(AssessedStatus::Red).fg, Some(theme.bad));
    assert_eq!(theme.status_style(AssessedStatus::Gray).fg, Some(theme.dim));
    assert_eq!(theme.border_style(true).fg, Some(theme.accent));
    assert_eq!(theme.border_style(false).fg, Some(theme.dim));
}

/// The save chip is filled rather than written, so it is the one thing on
/// the key bar that cannot be read as an ordinary key.
#[test]
fn the_chip_is_filled_and_the_column_headings_are_quiet() {
    let theme = Theme::default();
    assert_eq!(theme.chip_style().bg, Some(theme.warn));
    assert_eq!(theme.chip_style().fg, Some(Color::Black));
    assert!(theme.chip_style().add_modifier.contains(Modifier::BOLD));

    assert_eq!(theme.column_style().fg, Some(theme.dim));
    assert!(theme.column_style().add_modifier.contains(Modifier::BOLD));
    assert_eq!(theme.column_style().bg, None, "a heading is not a bar");
}
