//! Rendering: pure projection of [`crate::app::App`] state onto
//! ratatui widgets. No state lives here.

use pal_core::model::{Gender, PalDb, PalName, PassiveName};
use pal_solver::search::PlanNode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, Pane};

pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status, help] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [species, passives, results] = Layout::horizontal([
        Constraint::Percentage(24),
        Constraint::Percentage(26),
        Constraint::Percentage(50),
    ])
    .areas(main);

    draw_species(frame, app, species);
    draw_passives(frame, app, passives);
    draw_results(frame, app, results);
    frame.render_widget(Paragraph::new(app.status.as_str()), status);
    frame.render_widget(
        Paragraph::new(
            "Tab panes · type to filter · Enter select · F4 progenitor · Del clear · ←/→ depth · F2 wild · F5 search · Esc quit",
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
    let title = if app.progenitors.is_empty() {
        format!(" Pals — target: {target} ")
    } else {
        format!(
            " Pals — target: {target} · progenitors: {} ",
            app.progenitors.len()
        )
    };
    let rows = app.species_rows();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|pal| {
            if app.progenitors.contains(&pal.name) {
                ListItem::new(format!("[P] {}", pal.display_name))
            } else if app.progenitors.is_empty() {
                ListItem::new(pal.display_name.clone())
            } else {
                ListItem::new(format!("    {}", pal.display_name))
            }
        })
        .collect();
    draw_filterable_list(
        frame,
        area,
        &title,
        &app.species_filter,
        items,
        app.species_cursor,
        app.focus == Pane::Pals,
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
            ListItem::new(Line::from(vec![
                Span::raw(mark),
                Span::styled(skill.display_name.clone(), rank_style(skill.rank)),
            ]))
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
        |plan| plan_tree(app.db(), &plan.root),
    );
    frame.render_widget(
        Paragraph::new(detail).wrap(Wrap { trim: false }),
        detail_area,
    );
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let block = Block::bordered().title(title);
    if focused {
        block.border_style(Style::new().add_modifier(Modifier::BOLD))
    } else {
        block
    }
}

/// Family-tree rendering of a plan: the bred result on top, parents
/// as branches (♂ first), leaves tagged by where the pal comes from.
/// Passive names carry their in-game tier color (see [`rank_style`]).
///
/// ```text
/// 🥚 Fuack
/// ├─ ♂ 🎒 Lamball · Swift
/// ╰─ ♀ 🥚 Daedream · hatch for Swift
///    ├─ ♂ ⭐ Anubis · your progenitor
///    ╰─ ♀ 🌿 Cattiva · catch
/// ```
fn plan_tree(db: &PalDb, root: &PlanNode) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    tree_lines(db, root, "", "", None, &mut lines);
    lines
}

fn tree_lines(
    db: &PalDb,
    node: &PlanNode,
    prefix: &str,
    connector: &str,
    role: Option<Gender>,
    out: &mut Vec<Line<'static>>,
) {
    let role_glyph = match role {
        None => "",
        Some(Gender::Male) => "♂ ",
        Some(Gender::Female) => "♀ ",
    };
    let (icon, species, annotation) = match node {
        PlanNode::Owned(pal) => ("🎒", &pal.species, passive_spans(db, &pal.passives, "")),
        PlanNode::Wild(species) => ("🌿", species, vec![Span::raw(" · catch")]),
        PlanNode::Progenitor(species) => ("⭐", species, vec![Span::raw(" · your progenitor")]),
        PlanNode::Bred(bred) => (
            "🥚",
            &bred.species,
            passive_spans(db, &bred.carried_passives, "hatch for "),
        ),
    };
    let mut spans = vec![Span::raw(format!(
        "{prefix}{connector}{role_glyph}{icon} {}",
        display(db, species)
    ))];
    spans.extend(annotation);
    out.push(Line::from(spans));

    if let PlanNode::Bred(bred) = node {
        let child_prefix = format!(
            "{prefix}{}",
            match connector {
                "├─ " => "│  ",
                "╰─ " => "   ",
                _ => "",
            }
        );
        tree_lines(
            db,
            &bred.male,
            &child_prefix,
            "├─ ",
            Some(Gender::Male),
            out,
        );
        tree_lines(
            db,
            &bred.female,
            &child_prefix,
            "╰─ ",
            Some(Gender::Female),
            out,
        );
    }
}

/// ` · <verb>name, name, …` with every passive name styled by tier.
fn passive_spans(db: &PalDb, passives: &[PassiveName], verb: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (position, passive) in passives.iter().enumerate() {
        spans.push(Span::raw(if position == 0 {
            format!(" · {verb}")
        } else {
            ", ".to_owned()
        }));
        let style = db
            .passive(passive)
            .map_or_else(Style::new, |skill| rank_style(skill.rank));
        spans.push(Span::styled(display_passive(db, passive), style));
    }
    spans
}

/// The game's passive-tier palette: detrimental passives (negative
/// rank) red, regular tiers 1–3 gold, the special "rainbow" tier
/// (rank 4+) teal.
fn rank_style(rank: i8) -> Style {
    if rank < 0 {
        Style::new().fg(Color::Red)
    } else if rank >= 4 {
        Style::new().fg(Color::Cyan)
    } else if rank >= 1 {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;
    use pal_core::model::PassiveName;
    use pal_solver::search::{BredNode, OwnedPal};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn plan_tree_renders_family_style_with_leaf_tags() {
        let f = fixture();
        let plan = PlanNode::Bred(Box::new(BredNode {
            species: PalName::new("BluePlatypus"),
            carried_passives: vec![PassiveName::new("Swift")],
            male: PlanNode::Owned(OwnedPal {
                species: PalName::new("SheepBall"),
                gender: pal_core::model::Gender::Male,
                passives: vec![PassiveName::new("Swift")],
            }),
            female: PlanNode::Bred(Box::new(BredNode {
                species: PalName::new("DreamDemon"),
                carried_passives: Vec::new(),
                male: PlanNode::Progenitor(PalName::new("Anubis")),
                female: PlanNode::Wild(PalName::new("PinkCat")),
            })),
        }));

        let rendered: Vec<String> = plan_tree(f.db, &plan)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(
            rendered,
            vec![
                "🥚 Fuack · hatch for Swift".to_owned(),
                "├─ ♂ 🎒 Lamball · Swift".to_owned(),
                "╰─ ♀ 🥚 Daedream".to_owned(),
                "   ├─ ♂ ⭐ Anubis · your progenitor".to_owned(),
                "   ╰─ ♀ 🌿 Cattiva · catch".to_owned(),
            ]
        );
    }

    #[test]
    fn passive_tiers_get_their_in_game_colors() {
        let f = fixture();
        let by_rank = |predicate: fn(i8) -> bool| {
            f.db.passives()
                .find(|skill| skill.standard && predicate(skill.rank))
                .expect("data holds passives of every tier")
        };
        let detrimental = by_rank(|rank| rank < 0);
        let regular = by_rank(|rank| (1..=3).contains(&rank));
        let rainbow = by_rank(|rank| rank >= 4);

        assert_eq!(rank_style(detrimental.rank).fg, Some(Color::Red));
        assert_eq!(rank_style(regular.rank).fg, Some(Color::Yellow));
        assert_eq!(rank_style(rainbow.rank).fg, Some(Color::Cyan));

        // The tree renderer applies the tier color to the passive
        // span itself.
        let plan = PlanNode::Owned(OwnedPal {
            species: PalName::new("SheepBall"),
            gender: pal_core::model::Gender::Male,
            passives: vec![rainbow.name.clone()],
        });
        let lines = plan_tree(f.db, &plan);
        let rainbow_span = lines[0]
            .spans
            .iter()
            .find(|span| span.content == rainbow.display_name.as_str())
            .expect("passive span present");
        assert_eq!(rainbow_span.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn draws_all_panes_without_panicking() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new());
        app.species_filter = "lamb".to_owned();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Pals"));
        assert!(rendered.contains("Passives"));
        assert!(rendered.contains("Plans"));
        assert!(rendered.contains("depth"));
        assert!(rendered.contains("wild on"));
        assert!(rendered.contains("Lamball"));
        assert!(rendered.contains("/lamb"));
    }
}
