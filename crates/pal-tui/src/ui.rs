//! Rendering: pure projection of [`crate::app::App`] state onto
//! ratatui widgets. No state lives here.

use pal_core::model::{Gender, PalDb, PalName};
use pal_solver::search::PlanNode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::app::{App, Pane};

pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status, help] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [species, passives, results] = Layout::horizontal([
        Constraint::Percentage(28),
        Constraint::Percentage(30),
        Constraint::Percentage(42),
    ])
    .areas(main);

    draw_species(frame, app, species);
    draw_passives(frame, app, passives);
    draw_results(frame, app, results);
    frame.render_widget(Paragraph::new(app.status.as_str()), status);
    frame.render_widget(
        Paragraph::new(
            "Tab panes · type to filter · Enter select/toggle · ←/→ depth · F2 wild pals · F5 search · Esc quit",
        )
        .style(Style::new().add_modifier(Modifier::DIM)),
        help,
    );
}

fn draw_species(frame: &mut Frame, app: &App, area: Rect) {
    let target = app
        .target
        .as_ref()
        .map_or("none", |name| display(app.db(), name));
    let title = format!(" Species — target: {target} ");
    let rows = app.species_rows();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|pal| ListItem::new(pal.display_name.clone()))
        .collect();
    draw_filterable_list(
        frame,
        area,
        &title,
        &app.species_filter,
        items,
        app.species_cursor,
        app.focus == Pane::Species,
    );
}

fn draw_passives(frame: &mut Frame, app: &App, area: Rect) {
    let title = format!(" Passives ({}/4 picked) ", app.selected_passives.len());
    let items: Vec<ListItem> = app
        .passive_rows()
        .iter()
        .map(|skill| {
            let mark = if app.selected_passives.contains(&skill.name) {
                "[x] "
            } else {
                "[ ] "
            };
            ListItem::new(format!("{mark}{}", skill.display_name))
        })
        .collect();
    draw_filterable_list(
        frame,
        area,
        &title,
        &app.passive_filter,
        items,
        app.passive_cursor,
        app.focus == Pane::Passives,
    );
}

fn draw_filterable_list(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    filter: &str,
    items: Vec<ListItem>,
    cursor: usize,
    focused: bool,
) {
    let block = pane_block(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [filter_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    frame.render_widget(Paragraph::new(format!("/{filter}")), filter_area);
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn draw_results(frame: &mut Frame, app: &App, area: Rect) {
    let wild = if app.allow_wild { "on" } else { "off" };
    let title = format!(" Plans — depth ≤ {} · wild {wild} ", app.max_breeding_steps);
    let block = pane_block(&title, app.focus == Pane::Results);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [list_area, detail_area] =
        Layout::vertical([Constraint::Length(7), Constraint::Min(1)]).areas(inner);

    let items: Vec<ListItem> = app
        .plans
        .iter()
        .enumerate()
        .map(|(position, plan)| {
            ListItem::new(format!(
                "{}. {:.2} expected eggs, {} step(s)",
                position + 1,
                plan.expected_eggs,
                plan.steps
            ))
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(app.plan_cursor));
    frame.render_stateful_widget(list, list_area, &mut state);

    let detail: Vec<Line> = app.plans.get(app.plan_cursor).map_or_else(
        || vec![Line::from("run a search (F5) to see plans")],
        |plan| {
            let mut lines = Vec::new();
            plan_lines(app.db(), &plan.root, 0, &mut lines);
            lines.into_iter().map(Line::from).collect()
        },
    );
    frame.render_widget(Paragraph::new(detail), detail_area);
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let block = Block::bordered().title(title);
    if focused {
        block.border_style(Style::new().add_modifier(Modifier::BOLD))
    } else {
        block
    }
}

/// One indented line per plan node: breeding steps first, then the
/// pals they consume.
fn plan_lines(db: &PalDb, node: &PlanNode, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    match node {
        PlanNode::Wild(species) => {
            out.push(format!("{indent}catch {}", display(db, species)));
        }
        PlanNode::Owned(pal) => {
            let passives = if pal.passives.is_empty() {
                String::new()
            } else {
                format!(
                    " [{}]",
                    pal.passives
                        .iter()
                        .map(|p| display_passive(db, p))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            out.push(format!(
                "{indent}own   {} {}{passives}",
                display(db, &pal.species),
                gender_glyph(pal.gender),
            ));
        }
        PlanNode::Bred(node) => {
            let carry = if node.carried_passives.is_empty() {
                String::new()
            } else {
                format!(
                    " carrying [{}]",
                    node.carried_passives
                        .iter()
                        .map(|p| display_passive(db, p))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            out.push(format!(
                "{indent}breed {} = {} ♂ × {} ♀{carry}",
                display(db, &node.species),
                display(db, node.male.species()),
                display(db, node.female.species()),
            ));
            plan_lines(db, &node.male, depth + 1, out);
            plan_lines(db, &node.female, depth + 1, out);
        }
    }
}

fn display<'db>(db: &'db PalDb, name: &'db PalName) -> &'db str {
    db.pal(name)
        .map_or_else(|| name.as_str(), |pal| pal.display_name.as_str())
}

fn display_passive(db: &PalDb, name: &pal_core::model::PassiveName) -> String {
    db.passive(name)
        .map_or_else(|| name.as_str().to_owned(), |s| s.display_name.clone())
}

fn gender_glyph(gender: Gender) -> &'static str {
    match gender {
        Gender::Male => "♂",
        Gender::Female => "♀",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn draws_all_panes_without_panicking() {
        let f = fixture();
        let mut app = App::new(&f.db, &f.index, &f.odds, Vec::new());
        app.species_filter = "lamb".to_owned();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Species"));
        assert!(rendered.contains("Passives"));
        assert!(rendered.contains("Plans"));
        assert!(rendered.contains("depth"));
        assert!(rendered.contains("wild on"));
        assert!(rendered.contains("Lamball"));
        assert!(rendered.contains("/lamb"));
    }
}
