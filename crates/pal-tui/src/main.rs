//! Terminal frontend for the Palworld breeding calculator: pick a
//! target species and passives, search breeding plans over the pool
//! from `pals.toml` (optional first argument overrides the path).

mod app;
mod pals_file;
mod plan_store;
mod pool;
mod ui;

use anyhow::{Context, Result};
use pal_solver::child::ChildIndex;
use pal_solver::passives::PassiveOdds;
use pal_solver::search::Solver;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, KeyModifiers, MouseButton,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;

use crate::app::App;

fn main() -> Result<()> {
    let pals_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pals.toml".to_owned());

    let db = pal_core::vendored::pal_db().context("parsing the embedded db.json")?;
    let breeding =
        pal_core::vendored::breeding_db(&db).context("parsing the embedded breeding.json")?;
    let index = ChildIndex::build(&breeding).context("building the child index")?;
    let odds = PassiveOdds::from_mechanics(db.mechanics()).context("deriving passive odds")?;
    let solver = Solver::new(&db, &index, &odds);

    let (owned, pool_status) = match pool::load(&pals_path, &db)? {
        pool::Loaded::Pool { owned, status } => (owned, status),
        pool::Loaded::Missing => (
            Vec::new(),
            format!("{pals_path} not found — searching from an empty pool"),
        ),
    };

    let plans_path = plan_store::stable_path();
    let migrated = plan_store::migrate_legacy(std::path::Path::new("plans.json"), &plans_path)
        .unwrap_or(false);
    let (saved, store_note) = match plan_store::PlanStore::load(plans_path.clone()) {
        Ok(store) => {
            let note = if migrated {
                format!(
                    "; migrated {} saved plan(s) to {}",
                    store.plans.len(),
                    plans_path.display()
                )
            } else if store.plans.is_empty() {
                String::new()
            } else {
                format!(
                    "; {} saved plan(s) — F9 opens the library",
                    store.plans.len()
                )
            };
            (store, note)
        }
        Err(error) => (
            plan_store::PlanStore::fresh(plans_path),
            format!("; plans library issue: {error:#}"),
        ),
    };

    let mut app = App::new(&solver, owned, saved);
    app.pool_path = Some(pals_path);
    app.status = format!("{pool_status}{store_note}");

    let mut terminal = ratatui::init();
    let mouse_capture = execute!(std::io::stdout(), EnableMouseCapture).is_ok();
    let result = run(&mut terminal, &mut app);
    if mouse_capture {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let size = terminal.size()?;
                let area = Rect::new(0, 0, size.width, size.height);
                if let Some(click) = ui::locate_click(app, area, mouse.column, mouse.row) {
                    app.handle_click(click, mouse.modifiers.contains(KeyModifiers::SHIFT));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod test_support {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU32, Ordering};

    use pal_core::model::PalDb;
    use pal_solver::child::ChildIndex;
    use pal_solver::passives::PassiveOdds;
    use pal_solver::search::Solver;

    use crate::plan_store::PlanStore;

    /// A fresh store on a unique scratch path per call, so tests that
    /// persist never interfere with each other.
    pub fn test_store() -> PlanStore {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        PlanStore::fresh(std::env::temp_dir().join(format!(
            "pal-tui-test-store-{}-{unique}.json",
            std::process::id()
        )))
    }

    struct Data {
        db: PalDb,
        index: ChildIndex,
        odds: PassiveOdds,
    }

    pub struct Fixture {
        pub db: &'static PalDb,
        pub solver: &'static Solver<'static>,
    }

    pub fn fixture() -> Fixture {
        static DATA: OnceLock<Data> = OnceLock::new();
        static SOLVER: OnceLock<Solver<'static>> = OnceLock::new();
        let data = DATA.get_or_init(|| {
            let db = pal_core::vendored::pal_db().unwrap();
            let breeding = pal_core::vendored::breeding_db(&db).unwrap();
            let index = ChildIndex::build(&breeding).unwrap();
            let odds = PassiveOdds::from_mechanics(db.mechanics()).unwrap();
            Data { db, index, odds }
        });
        let solver = SOLVER.get_or_init(|| Solver::new(&data.db, &data.index, &data.odds));
        Fixture {
            db: &data.db,
            solver,
        }
    }
}
