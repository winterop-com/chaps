use super::*;
use ratatui::layout::Rect;
use ratatui::style::Style;
use std::time::Duration;

fn buffer(width: u16, height: u16) -> Buffer {
    Buffer::empty(Rect::new(0, 0, width, height))
}

#[test]
fn a_buffer_becomes_a_picture_of_itself() {
    let mut buffer = buffer(20, 2);
    buffer.set_string(0, 0, "chaps · models", Style::default());
    buffer.set_string(0, 1, "7 models", Style::default().fg(Color::Cyan));
    let svg = svg(&buffer, &Theme::default());

    assert!(
        svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""),
        "{svg}"
    );
    assert!(svg.ends_with("</svg>\n"), "{svg}");
    // Twenty cells by two rows, at the cell size the font is set for.
    assert!(svg.contains("width=\"168\" height=\"36\""), "{svg}");
    // The ground is one rect, before anything is drawn on it.
    assert!(
        svg.contains(&format!(
            "<rect width=\"168\" height=\"36\" fill=\"{}\"/>",
            hex(Theme::default().background(), "#141618")
        )),
        "{svg}"
    );
    assert!(svg.contains(">chaps · models</tspan>"), "{svg}");
    assert!(
        svg.contains("fill=\"#06989a\">7 models</tspan>"),
        "cyan: {svg}"
    );
    // One text element per row that has anything on it.
    assert_eq!(svg.matches("<text").count(), 2, "{svg}");
}

#[test]
fn the_three_characters_that_would_break_the_file_are_escaped() {
    let mut buffer = buffer(24, 1);
    buffer.set_string(0, 0, "a <b> & \"c\"", Style::default());
    let svg = svg(&buffer, &Theme::default());
    assert!(svg.contains("a &lt;b&gt; &amp; &quot;c&quot;"), "{svg}");
    assert!(!svg.contains("<b>"), "{svg}");
}

#[test]
fn weight_colour_and_reverse_video_survive_the_trip() {
    let mut styled = buffer(24, 1);
    styled.set_string(0, 0, "MODEL", Theme::default().label_style());
    styled.set_string(6, 0, "chaps", Theme::default().accent_style());
    styled.set_string(12, 0, "dim", Theme::default().dim_style());
    let styled = svg(&styled, &Theme::default());
    assert!(
        styled.contains("fill=\"#d8dce0\" font-weight=\"bold\">MODEL</tspan>"),
        "a heading is bold in the terminal's own ink: {styled}"
    );
    assert!(
        styled.contains("fill=\"#06989a\">chaps</tspan>"),
        "the accent is cyan: {styled}"
    );
    assert!(
        styled.contains("fill=\"#555753\">dim</tspan>"),
        "dim is a colour here, not an opacity: {styled}"
    );

    // Reverse video has no terminal to ask, so it is drawn as the swap it
    // is: the row's colour becomes the ground under it.
    let mut reversed = buffer(12, 1);
    reversed.set_string(0, 0, "row", Theme::monochrome().selection_style());
    let reversed = svg(&reversed, &Theme::monochrome());
    assert!(
        reversed.contains("fill=\"#141618\">row</tspan>"),
        "the ground became the ink: {reversed}"
    );
    assert!(
        reversed.contains("<rect x=\"0\" y=\"0\" width=\"25.2\" height=\"18\" fill=\"#d8dce0\"/>"),
        "and the ink became the ground, painted under it: {reversed}"
    );
}

/// A row of nothing is not worth an element, and the picture of an empty
/// screen is still a valid document.
#[test]
fn blank_rows_draw_nothing() {
    let svg = svg(&buffer(10, 3), &Theme::default());
    assert!(!svg.contains("<text"), "{svg}");
    assert_eq!(svg.matches("<rect").count(), 1, "only the ground: {svg}");
}

#[test]
fn the_file_is_named_after_the_second_it_was_taken_in() {
    // 2026-09-25T14:30:12Z.
    let now = UNIX_EPOCH + Duration::from_secs(1_790_346_612);
    assert_eq!(file_name(now), "chaps-ui-20260925-143012.svg");
    assert_eq!(stamp(0), "19700101-000000");
    // A leap day, which is where a home-made calendar goes wrong.
    assert_eq!(stamp(1_709_164_800), "20240229-000000");
}

#[test]
fn saving_writes_the_file_and_says_where() {
    let dir = tempfile::tempdir().unwrap();
    let mut buffer = buffer(16, 1);
    buffer.set_string(0, 0, "chaps", Style::default());
    let now = UNIX_EPOCH + Duration::from_secs(1_790_346_612);

    let message = save_in(dir.path(), &buffer, &Theme::default(), now);
    assert_eq!(message, "saved chaps-ui-20260925-143012.svg");
    let written = std::fs::read_to_string(dir.path().join("chaps-ui-20260925-143012.svg")).unwrap();
    assert!(written.contains(">chaps</tspan>"), "{written}");

    // A directory that is not there is a message, not a panic, and
    // nothing is created to make room for the file.
    let missing = dir.path().join("nope");
    let message = save_in(&missing, &buffer, &Theme::default(), now);
    assert!(
        message.starts_with("could not save `chaps-ui-"),
        "{message}"
    );
    assert!(!missing.exists(), "no directory was made");
}

/// A picture of the browser is a picture of whatever page was up: the
/// buffer is the only thing this reads, so the components page needs no
/// special case - which is worth a test rather than a comment.
#[test]
fn the_components_page_photographs_like_the_model_page() {
    use crate::project::ProjectState;
    use crate::tui::app::{Action, App};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let registry = crate::registry::load_embedded().expect("embedded snapshot parses");
    let mut state = ProjectState::default();
    state
        .components
        .set_enabled(crate::components::Component::Ocs, true);
    let mut app = App::new(&registry, &state);
    app.reduce(Action::NextPage);

    let mut terminal = Terminal::new(TestBackend::new(120, 12)).expect("test backend starts");
    terminal
        .draw(|frame| crate::tui::ui::draw(frame, &app, &Theme::default()))
        .expect("draw");
    let svg = svg(terminal.backend().buffer(), &Theme::default());

    assert!(svg.starts_with("<svg "), "{svg}");
    // The title bar paints each part in its own colour, so the page's name
    // is a span of its own.
    assert!(svg.contains(">components</tspan>"), "{svg}");
    assert!(svg.contains("COMPONENT    STATE    REACH"), "{svg}");
    assert!(svg.contains("chap-core"), "{svg}");
    assert!(svg.contains(">http://localhost:8790</tspan>"), "{svg}");
    assert!(svg.ends_with("</svg>\n"), "{svg}");
}

#[test]
fn every_kind_of_colour_becomes_a_hex_value() {
    assert_eq!(hex(Color::Reset, "#123456"), "#123456");
    assert_eq!(hex(Color::Rgb(1, 2, 3), "#000000"), "#010203");
    assert_eq!(hex(Color::Green, "#000000"), ANSI[2]);
    assert_eq!(hex(Color::Indexed(9), "#000000"), ANSI[9]);
    // The middle of the 6x6x6 cube, and the top of the grey ramp.
    assert_eq!(hex(Color::Indexed(16), "#000000"), "#000000");
    assert_eq!(hex(Color::Indexed(231), "#000000"), "#ffffff");
    assert_eq!(hex(Color::Indexed(255), "#000000"), "#eeeeee");
}
