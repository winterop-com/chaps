use super::*;
use crate::configs::{self, Draft};
use crate::project::ProjectState;
use crate::registry::{Registry, load_embedded};
use crate::tui::app::configs::form_for;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;

fn registry() -> Registry {
    load_embedded().expect("embedded snapshot parses")
}

fn render(app: &App, width: u16, height: u16) -> String {
    let theme = Theme::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal
        .draw(|frame| crate::tui::ui::draw(frame, app, &theme))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| {
                    buffer
                        .cell((x, y))
                        .map(|c| c.symbol().to_string())
                        .unwrap_or_default()
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn template() -> configs::Template {
    configs::parse_template(&json!({
        "id": 30,
        "name": "chapkit-ewars-model",
        "allowFreeAdditionalContinuousCovariates": true,
        "userOptions": {
            "n_lags": {"type": "integer", "default": 3, "description": "Lags of the target."}
        }
    }))
    .expect("a template")
}

fn view(load: Load) -> View {
    View {
        model: "chapkit_ewars_model".to_string(),
        service: "chapkit-ewars-model".to_string(),
        display_name: "CHAP-EWARS".to_string(),
        load,
        cursor: 0,
        form: None,
        pending: None,
    }
}

fn ready() -> Load {
    let rows = json!([
        {"id": 7, "name": "chapkit-ewars-model:monthly_climate",
         "additionalContinuousCovariates": ["rainfall"], "userOptionValues": {"n_lags": 6}},
        {"id": 8, "name": "chapkit-ewars-model:weekly"}
    ]);
    Load::Ready {
        template: Some(template()),
        configs: configs::configs_of(&rows, "chapkit-ewars-model")
            .into_iter()
            .map(|config| (config, "marketplace"))
            .collect(),
    }
}

#[test]
fn the_page_lists_the_configured_models_with_the_keys_under_it() {
    let registry = registry();
    let mut app = App::new(&registry, &ProjectState::default());
    app.configs = Some(view(ready()));
    app.mode = Mode::Configs;
    let screen = render(&app, 120, 30);
    assert!(
        screen.contains("Configured models · CHAP-EWARS · chapkit_ewars_model"),
        "{screen}"
    );
    let row = screen
        .lines()
        .find(|line| line.contains("monthly_climate"))
        .expect("a row");
    assert!(row.contains("rainfall"), "{row}");
    assert!(row.contains("marketplace"), "{row}");
    assert!(row.contains("n_lags=6"), "{row}");
    assert!(screen.contains("weekly"), "{screen}");
    let footer = screen.lines().last().expect("a footer");
    assert!(footer.contains("[a] add"), "{footer}");
    assert!(footer.contains("[d] archive"), "{footer}");
    assert!(footer.contains("[esc] back"), "{footer}");
}

#[test]
fn chap_core_down_is_said_with_the_way_out() {
    let registry = registry();
    let mut app = App::new(&registry, &ProjectState::default());
    app.configs = Some(view(Load::Failed(
        "chap-core at http://localhost:8000 is not running; start it with `varde up`, then \
         press r"
            .to_string(),
    )));
    app.mode = Mode::Configs;
    let screen = render(&app, 120, 30);
    assert!(screen.contains("start it with `varde up`"), "{screen}");
}

#[test]
fn the_form_shows_each_field_its_default_and_what_it_takes() {
    let mut form = form_for(&template());
    form.cursor = 1;
    let lines: Vec<String> = form_lines(&form, 70, &Theme::default())
        .iter()
        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    assert!(lines[0].contains("name"), "{lines:?}");
    assert!(lines[1].starts_with(" ▸ n_lags"), "{lines:?}");
    assert!(lines[1].contains("3_"), "{lines:?}");
    assert!(lines[2].contains("covariates"), "{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("integer. Lags of the target.")),
        "{lines:?}"
    );
}

#[test]
fn the_question_says_what_is_written_and_what_archive_means() {
    let mut pending = view(ready());
    pending.pending = Some(Pending::Create(Draft {
        variant: "weekly2".to_string(),
        values: serde_json::Map::new(),
        covariates: vec!["rainfall".to_string()],
    }));
    let (title, lines) = question(&pending);
    assert_eq!(title, "Create");
    assert_eq!(
        lines,
        [
            "Create the configured model weekly2 of chapkit_ewars_model in chap-core?",
            "covariates: rainfall"
        ]
    );
    pending.pending = Some(Pending::Archive {
        id: 7,
        variant: "weekly".to_string(),
    });
    let (title, lines) = question(&pending);
    assert_eq!(title, "Archive");
    assert_eq!(
        lines[1],
        "chap-core keeps it, and the Modeling App shows it as Archived."
    );
}
