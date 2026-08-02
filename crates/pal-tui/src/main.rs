//! Terminal frontend for the Palworld breeding calculator: pick a
//! target species and passives, search breeding plans over the pool
//! from `pals.toml` (optional first argument overrides the path).

mod app;
mod pals_file;
mod ui;

use anyhow::{Context, Result};
use pal_solver::child::ChildIndex;
use pal_solver::passives::PassiveOdds;
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

    let (owned, pool_status) = match std::fs::read_to_string(&pals_path) {
        Ok(text) => {
            let owned = pals_file::parse(&text, &db).with_context(|| format!("in {pals_path}"))?;
            let status = format!("{} owned pal(s) loaded from {pals_path}", owned.len());
            (owned, status)
        }
        Err(_) => (
            Vec::new(),
            format!("{pals_path} not found — searching from an empty pool"),
        ),
    };

    let mut app = App::new(&db, &index, &odds, owned);
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

    pub struct Fixture {
        pub db: PalDb,
        pub index: ChildIndex,
        pub odds: PassiveOdds,
    }

    pub fn fixture() -> &'static Fixture {
        static FIXTURE: OnceLock<Fixture> = OnceLock::new();
        FIXTURE.get_or_init(|| {
            let db = pal_core::vendored::pal_db().unwrap();
            let breeding = pal_core::vendored::breeding_db(&db).unwrap();
            let index = ChildIndex::build(&breeding).unwrap();
            let odds = PassiveOdds::from_mechanics(db.mechanics()).unwrap();
            Fixture { db, index, odds }
        })
    }
}
