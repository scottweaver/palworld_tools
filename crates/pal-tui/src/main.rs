//! Terminal frontend for the Palworld breeding calculator: pick a
//! target species and passives, search breeding plans over the pool
//! from `pals.toml` (optional first argument overrides the path).

mod app;
mod pals_file;
mod ui;

use anyhow::{Context, Result};
use pal_solver::child::ChildIndex;
use pal_solver::passives::PassiveOdds;
use pal_solver::search::Solver;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

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

    let (owned, pool_status) = match std::fs::read(&pals_path) {
        Ok(bytes) if pal_save::looks_like_sav(&bytes) => {
            let save = pal_save::level::read_level_sav(&bytes)
                .with_context(|| format!("parsing save file {pals_path}"))?;
            let report = pal_save::import::import_pals(&db, &save.characters);
            let status = format!(
                "{} pal(s) imported from {pals_path} ({} player(s), {} other entries skipped)",
                report.pals.len(),
                report.skipped_players(),
                report.skipped.len() - report.skipped_players(),
            );
            let owned = report
                .pals
                .into_iter()
                .map(|pal| pal_solver::search::OwnedPal {
                    species: pal.species,
                    gender: pal.gender,
                    passives: pal.passives,
                })
                .collect();
            (owned, status)
        }
        Ok(bytes) => {
            let text = String::from_utf8(bytes)
                .with_context(|| format!("{pals_path} is neither a save file nor UTF-8 TOML"))?;
            let owned = pals_file::parse(&text, &db).with_context(|| format!("in {pals_path}"))?;
            let status = format!("{} owned pal(s) loaded from {pals_path}", owned.len());
            (owned, status)
        }
        Err(_) => (
            Vec::new(),
            format!("{pals_path} not found — searching from an empty pool"),
        ),
    };

    let mut app = App::new(&solver, owned);
    app.status = pool_status;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
    }
    Ok(())
}

#[cfg(test)]
mod test_support {
    use std::sync::OnceLock;

    use pal_core::model::PalDb;
    use pal_solver::child::ChildIndex;
    use pal_solver::passives::PassiveOdds;
    use pal_solver::search::Solver;

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
