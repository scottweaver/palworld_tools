//! Rendering: pure projection of [`crate::app::App`] state onto
//! ratatui widgets. No state lives here.

use pal_core::model::{Gender, IvValue, PalDb, PalName, PassiveName};
use pal_solver::iv::IvThresholds;
use pal_solver::search::PlanNode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, Click, Overlay, Pane};
use crate::command::DocView;

/// The screen regions draw and click hit-testing must agree on.
struct PaneAreas {
    species: Rect,
    passives: Rect,
    results: Rect,
    status: Rect,
    help: Rect,
}

fn pane_areas(area: Rect) -> PaneAreas {
    let [main, status, help] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [species, passives, results] = Layout::horizontal([
        Constraint::Percentage(24),
        Constraint::Percentage(26),
        Constraint::Percentage(50),
    ])
    .areas(main);
    PaneAreas {
        species,
        passives,
        results,
        status,
        help,
    }
}

/// Rows the plans list occupies inside the Results pane (the detail
/// tree renders below).
const PLANS_LIST_HEIGHT: u16 = 7;

pub fn draw(frame: &mut Frame, app: &App) {
    let areas = pane_areas(frame.area());

    draw_species(frame, app, areas.species);
    draw_passives(frame, app, areas.passives);
    draw_results(frame, app, areas.results);
    frame.render_widget(status_line(app), areas.status);
    frame.render_widget(
        Paragraph::new(
            "Tab panes · type filter · Enter/click select · F4/⇧click progenitor · ^D clear/delete · ←/→ depth · h/a/d IV min (⇧ lowers) · F2 wild · F5 search · F6 reload pool · F8 save plan · F9 library · : commands (:help) · Esc quit",
        )
        .style(Style::new().add_modifier(Modifier::DIM)),
        areas.help,
    );
    if let Overlay::Doc(view) = &app.overlay {
        draw_doc(frame, view, frame.area());
    }
}

/// The bottom status line doubles as the vim-style command line:
/// while the prompt is open it shows the buffer with a block cursor
/// in place of the status text.
fn status_line<'app>(app: &'app App) -> Paragraph<'app> {
    match &app.overlay {
        Overlay::Prompt(buffer) => Paragraph::new(Line::from(vec![
            Span::raw(format!(":{buffer}")),
            Span::styled(" ", Style::new().add_modifier(Modifier::REVERSED)),
        ])),
        Overlay::None | Overlay::Doc(_) => Paragraph::new(app.status.as_str()),
    }
}

/// Full-screen viewer for `:help` / `:readme`: the embedded markdown
/// rendered over the panes, unwrapped so tables keep their shape.
fn draw_doc(frame: &mut Frame, view: &DocView, area: Rect) {
    frame.render_widget(Clear, area);
    let block = pane_block(view.doc.title(), true)
        .title_bottom(" ↑/↓/wheel scroll · PgUp/PgDn · Home/End · Esc/q close ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(tui_markdown::from_str(view.doc.markdown())).scroll((view.scroll, 0)),
        inner,
    );
}

/// Maps a click position to the pane (and list row) it landed on.
/// Must mirror the geometry [`draw`] renders, including the scroll
/// offset ratatui applies to keep the cursor visible.
pub fn locate_click(app: &App, area: Rect, column: u16, row: u16) -> Option<Click> {
    let areas = pane_areas(area);
    let position = Position { x: column, y: row };

    if areas.species.contains(position) {
        let index = filterable_row_at(
            areas.species,
            position,
            app.species_cursor,
            app.species_rows().len(),
        );
        return Some(Click {
            pane: Pane::Pals,
            row: index,
        });
    }
    if areas.passives.contains(position) {
        let index = filterable_row_at(
            areas.passives,
            position,
            app.passive_cursor,
            app.passive_rows().len(),
        );
        return Some(Click {
            pane: Pane::Passives,
            row: index,
        });
    }
    if areas.results.contains(position) {
        let inner = areas.results.inner(Margin::new(1, 1));
        let list_bottom = inner.y.saturating_add(PLANS_LIST_HEIGHT.min(inner.height));
        let index = (inner.contains(position) && position.y < list_bottom)
            .then(|| {
                row_index(
                    inner.y,
                    position.y,
                    app.plan_cursor,
                    PLANS_LIST_HEIGHT.min(inner.height),
                    app.plans.len(),
                )
            })
            .flatten();
        return Some(Click {
            pane: Pane::Results,
            row: index,
        });
    }
    None
}

/// Row index within a filter-plus-list pane (filter line first, list
/// below the borders).
fn filterable_row_at(
    pane: Rect,
    position: Position,
    cursor: usize,
    rows_len: usize,
) -> Option<usize> {
    let inner = pane.inner(Margin::new(1, 1));
    if !inner.contains(position) || position.y == inner.y {
        return None;
    }
    let list_top = inner.y + 1;
    let list_height = inner.height.saturating_sub(1);
    row_index(list_top, position.y, cursor, list_height, rows_len)
}

fn row_index(
    list_top: u16,
    click_y: u16,
    cursor: usize,
    list_height: u16,
    rows_len: usize,
) -> Option<usize> {
    let offset = scroll_offset(cursor, usize::from(list_height));
    let index = offset + usize::from(click_y.checked_sub(list_top)?);
    (index < rows_len).then_some(index)
}

/// The offset ratatui's `List` settles on when the state is rebuilt
/// each frame with only `selected` set: scroll just far enough to
/// keep the selection visible.
fn scroll_offset(selected: usize, height: usize) -> usize {
    if height == 0 {
        0
    } else {
        selected.saturating_sub(height - 1)
    }
}

fn draw_species(frame: &mut Frame, app: &App, area: Rect) {
    let title = if app.progenitors.is_empty() {
        " Pals ".to_owned()
    } else {
        format!(" Pals — progenitors: {} ", app.progenitors.len())
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
    let (title, items, cursor, detail): (String, Vec<ListItem>, usize, Vec<Line>) = if app
        .viewing_saved
    {
        let title = format!(" Saved plans ({}) ", app.saved.plans.len());
        let items = app
            .saved
            .plans
            .iter()
            .map(|plan| ListItem::new(plan.label.clone()))
            .collect();
        let detail = app.saved.plans.get(app.saved_cursor).map_or_else(
                || vec![Line::from("library empty — F8 saves the highlighted plan")],
                |plan| {
                    let stale = app.stale_leaves(plan);
                    let banner = if stale > 0 {
                        Line::from(Span::styled(
                            format!(
                                "⚠ {stale} pal(s) no longer in your box — Enter re-plans with the current box"
                            ),
                            Style::new().fg(Color::Yellow),
                        ))
                    } else {
                        Line::from(Span::styled(
                            "✓ all pals still available · Enter re-plans with the current box",
                            Style::new().add_modifier(Modifier::DIM),
                        ))
                    };
                    let mut lines = vec![banner];
                    lines.extend(plan_tree(
                        app.db(),
                        &plan.root,
                        !plan.goal_iv_thresholds.is_empty(),
                    ));
                    lines
                },
            );
        (title, items, app.saved_cursor, detail)
    } else {
        let wild = if app.allow_wild { "on" } else { "off" };
        let target = app
            .target
            .as_ref()
            .map_or("no target", |name| display(app.db(), name));
        let title = format!(
            " Plans — {target} · depth ≤ {} · wild {wild}{} ",
            app.max_breeding_steps,
            iv_title(app.iv_thresholds)
        );
        let items = app
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
        let detail = app.plans.get(app.plan_cursor).map_or_else(
            || vec![Line::from("run a search (F5) to see plans")],
            |plan| plan_tree(app.db(), &plan.root, !app.iv_thresholds.is_empty()),
        );
        (title, items, app.plan_cursor, detail)
    };

    let block = pane_block(&title, app.focus == Pane::Results);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [list_area, detail_area] =
        Layout::vertical([Constraint::Length(PLANS_LIST_HEIGHT), Constraint::Min(1)]).areas(inner);

    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(cursor));
    frame.render_stateful_widget(list, list_area, &mut state);

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
fn plan_tree(db: &PalDb, root: &PlanNode, show_ivs: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    tree_lines(db, root, show_ivs, "", "", None, &mut lines);
    lines
}

/// Title segment for active IV minimums, hp/attack/defense order,
/// `–` for stats without one; empty when none are set.
fn iv_title(thresholds: IvThresholds) -> String {
    if thresholds.is_empty() {
        return String::new();
    }
    let part = |threshold: Option<IvValue>| {
        threshold.map_or_else(|| "–".to_owned(), |minimum| minimum.get().to_string())
    };
    format!(
        " · IV ≥ {}/{}/{}",
        part(thresholds.hp),
        part(thresholds.attack),
        part(thresholds.defense)
    )
}

fn tree_lines(
    db: &PalDb,
    node: &PlanNode,
    show_ivs: bool,
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
        PlanNode::Owned(pal) => {
            let mut spans = passive_spans(db, &pal.passives, "");
            if show_ivs {
                spans.push(Span::styled(
                    format!(
                        " · IVs {}/{}/{}",
                        pal.ivs.hp.get(),
                        pal.ivs.attack.get(),
                        pal.ivs.defense.get()
                    ),
                    Style::new().add_modifier(Modifier::DIM),
                ));
            }
            ("🎒", &pal.species, spans)
        }
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
            show_ivs,
            &child_prefix,
            "├─ ",
            Some(Gender::Male),
            out,
        );
        tree_lines(
            db,
            &bred.female,
            show_ivs,
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
    use crate::test_support::{fixture, test_store};
    use pal_core::model::{IvSpread, PassiveName};
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
                ivs: IvSpread::default(),
            }),
            female: PlanNode::Bred(Box::new(BredNode {
                species: PalName::new("DreamDemon"),
                carried_passives: Vec::new(),
                male: PlanNode::Progenitor(PalName::new("Anubis")),
                female: PlanNode::Wild(PalName::new("PinkCat")),
            })),
        }));

        let rendered: Vec<String> = plan_tree(f.db, &plan, false)
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
    fn scroll_offset_keeps_the_selection_visible() {
        assert_eq!(scroll_offset(0, 5), 0);
        assert_eq!(scroll_offset(4, 5), 0);
        assert_eq!(scroll_offset(5, 5), 1);
        assert_eq!(scroll_offset(10, 3), 8);
        assert_eq!(scroll_offset(10, 0), 0);
    }

    #[test]
    fn clicks_resolve_rows_through_the_layout() {
        let f = fixture();
        let app = App::new(f.solver, Vec::new(), test_store());
        let area = Rect::new(0, 0, 120, 40);
        let areas = pane_areas(area);

        // First list row of the Pals pane (one below the filter line).
        let inner = areas.species.inner(Margin::new(1, 1));
        assert_eq!(
            locate_click(&app, area, inner.x, inner.y + 1),
            Some(Click {
                pane: Pane::Pals,
                row: Some(0),
            })
        );
        // The filter line focuses the pane without selecting a row.
        assert_eq!(
            locate_click(&app, area, inner.x, inner.y),
            Some(Click {
                pane: Pane::Pals,
                row: None,
            })
        );
        // With no plans, a Results-list click is focus-only.
        let results_inner = areas.results.inner(Margin::new(1, 1));
        assert_eq!(
            locate_click(&app, area, results_inner.x, results_inner.y),
            Some(Click {
                pane: Pane::Results,
                row: None,
            })
        );
        // The status bar is nobody's pane.
        assert_eq!(locate_click(&app, area, 0, 38), None);
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
            ivs: IvSpread::default(),
        });
        let lines = plan_tree(f.db, &plan, false);
        let rainbow_span = lines[0]
            .spans
            .iter()
            .find(|span| span.content == rainbow.display_name.as_str())
            .expect("passive span present");
        assert_eq!(rainbow_span.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn prompt_and_doc_overlays_render_over_the_panes() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        app.overlay = Overlay::Prompt("hel".to_owned());
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains(":hel"));

        app.overlay = Overlay::Doc(crate::command::DocView::open(crate::command::Doc::Help));
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Help"));
        assert!(rendered.contains("Esc/q close"));
        assert!(
            !rendered.contains("Passives ("),
            "the viewer covers the panes"
        );
    }

    #[test]
    fn draws_all_panes_without_panicking() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        app.species_filter = "lamb".to_owned();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Pals"));
        assert!(rendered.contains("Passives"));
        assert!(rendered.contains("Plans"));
        assert!(rendered.contains("no target"));
        assert!(rendered.contains("depth"));
        assert!(rendered.contains("wild off"));
        assert!(rendered.contains("Lamball"));
        assert!(rendered.contains("/lamb"));
    }
}
