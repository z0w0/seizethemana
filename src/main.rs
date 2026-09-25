use clap::Parser;
use rusqlite::Connection;
use seizethemana::{
    card,
    cli::{self, Cli, CollectionCommand, Command, DeckCommand, codes},
    collection, collection_conflicts, collection_stats, db, deck,
    output::Output,
    paths, query, setup, sync,
};

fn main() {
    let cli = cli::Cli::parse();
    seizethemana::set_offline(cli.offline);
    let mut out = Output::new(cli.json_flag(), cli.no_color, cli.verbose);
    let code = run(&cli, &mut out);
    std::process::exit(code);
}

/// Dispatch a parsed command, returning the process exit code.
fn run(cli: &Cli, out: &mut Output) -> i32 {
    let paths = match paths::Paths::resolve(cli.data_dir.as_deref()) {
        Ok(paths) => paths,
        Err(err) => {
            out.error(&format!("{err:#}"));
            return codes::ERROR;
        }
    };

    // Bootstrap storage layout first, then the database inside it.
    if let Err(err) = paths.ensure_dirs() {
        out.error(&format!("{err:#}"));
        return codes::ERROR;
    }
    let mut conn = match db::open(&paths.db()) {
        Ok(conn) => conn,
        Err(err) => {
            out.error(&format!("{err:#}"));
            return codes::ERROR;
        }
    };

    // Result-of-Result: the inner Result is the command's own failure, the
    // outer is its exit-code decision.
    let outcome = dispatch(cli, &paths, &mut conn, out);
    outcome.unwrap_or_else(|err| {
        // A bail carrying the sentinel already printed its own error line;
        // re-reporting it would double the message.
        if err.to_string() != seizethemana::output::SILENT_ERROR {
            out.error(&format!("{err:#}"));
        }
        codes::ERROR
    })
}

/// Command-to-handler dispatch table.
fn dispatch(
    cli: &Cli,
    paths: &paths::Paths,
    conn: &mut Connection,
    out: &mut Output,
) -> anyhow::Result<i32> {
    match &cli.command {
        Command::Setup { force } => setup::run_setup(paths, out, conn, *force),
        Command::Sync { force } => {
            let options = sync::SyncOptions { force: *force };
            sync::run_sync(paths, conn, out, &options).map_err(|err| err.context("sync failed"))
        }
        Command::Card {
            command,
            name,
            json,
        } => run_card_group(paths, conn, out, command, name.as_deref(), *json),
        Command::Query {
            query,
            filters,
            max_price,
            limit,
            json,
            ..
        } => {
            revalidate_if_stale(paths, conn, out);
            query::run_query(paths, conn, out, query, filters, *max_price, *limit, *json)
        }
        Command::Collection { json, command, .. } => {
            run_collection(paths, conn, out, *json, command)
        }
        Command::Deck {
            json,
            command,
            name,
        } => run_deck(paths, conn, out, *json, command, name.as_deref()),
    }
}

/// Card subcommand dispatch with the shared staleness revalidation.
fn run_card_group(
    paths: &paths::Paths,
    conn: &mut Connection,
    out: &mut Output,
    command: &Option<cli::CardCommand>,
    name: Option<&str>,
    json: bool,
) -> anyhow::Result<i32> {
    let mut revalidate = |out: &mut Output| revalidate_if_stale(paths, conn, out);
    match command {
        Some(cli::CardCommand::Show { name, json }) => {
            revalidate(out);
            card::run_card(paths, conn, out, name, *json)
        }
        Some(cli::CardCommand::Similar {
            name,
            limit,
            owned,
            json,
        }) => {
            revalidate(out);
            card::similar::run_similar(paths, conn, out, name, *limit, *owned, *json)
        }
        Some(cli::CardCommand::Combos {
            name,
            format,
            limit,
            json,
        }) => {
            revalidate(out);
            card::combos::run_combos(paths, conn, out, name, format.as_deref(), *limit, *json)
        }
        None => {
            let Some(name) = name else {
                out.error("card name is required");
                out.hint("use 'stm card <name>' or 'stm card show <name>'");
                return Ok(codes::USAGE);
            };
            revalidate(out);
            card::run_card(paths, conn, out, name, json)
        }
    }
}

/// Dispatch collection subcommands.
fn run_collection(
    paths: &paths::Paths,
    conn: &mut Connection,
    out: &mut Output,
    json: bool,
    command: &Option<CollectionCommand>,
) -> anyhow::Result<i32> {
    match command {
        None => collection_stats::show_stats(paths, conn, out, json),
        Some(CollectionCommand::Conflicts { json }) => {
            collection_conflicts::conflicts(paths, conn, out, *json)
        }
        Some(CollectionCommand::Import { file, add }) => {
            collection::import(paths, conn, out, file, *add)
        }
        Some(CollectionCommand::Query {
            query,
            filters,
            binder,
            deck,
            limit,
            json,
            ..
        }) => {
            revalidate_if_stale(paths, conn, out);
            collection::run_query(
                paths, conn, out, query, filters, binder, deck, *limit, *json,
            )
        }
    }
}

/// Dispatch deck subcommands.
fn run_deck(
    paths: &paths::Paths,
    conn: &mut Connection,
    out: &mut Output,
    json: bool,
    command: &Option<DeckCommand>,
    sugar_name: Option<&str>,
) -> anyhow::Result<i32> {
    let outcome = match command {
        None => match sugar_name {
            Some(name) => deck::show(paths, conn, out, name, json),
            None => deck::list(paths, conn, out, json),
        },
        Some(DeckCommand::Create { name }) => deck::create(paths, out, name),
        Some(DeckCommand::List { json }) => deck::list(paths, conn, out, *json),
        Some(DeckCommand::Show { name, json }) => match name {
            Some(name) => deck::show(paths, conn, out, name, *json),
            None => deck::list(paths, conn, out, *json),
        },
        Some(DeckCommand::Mana { name, format, json }) => {
            deck::mana(paths, conn, out, name, format.as_deref(), *json)
        }
        Some(DeckCommand::Hand {
            name,
            seed,
            count,
            json,
        }) => deck::hand(paths, conn, out, name, *seed, *count, *json),
        Some(DeckCommand::Update {
            name,
            add,
            remove,
            set,
            r#move,
            from,
            allow_partial,
            dry_run,
            sim,
            legal,
            backfill_basics,
            json,
        }) => deck::update(
            paths,
            conn,
            out,
            name,
            add,
            remove,
            set,
            r#move,
            from.as_deref(),
            *allow_partial,
            *dry_run,
            *sim,
            *legal,
            *backfill_basics,
            *json,
        ),
        Some(DeckCommand::Dedupe { name, json }) => {
            deck::maintain::dedupe(paths, conn, out, name, *json)
        }
        Some(DeckCommand::Suggest {
            name,
            positional_query,
            query,
            role,
            commander,
            format,
            bracket,
            max_price,
            limit,
            owned,
            exclude,
            json,
        }) => {
            // The positional query and --query alias; positional wins only
            // when the flag is absent.
            let effective_query = query.clone().or_else(|| positional_query.clone());
            deck::suggest::suggest(
                paths,
                conn,
                out,
                name,
                effective_query.as_deref(),
                role.as_deref(),
                *commander,
                format.as_deref(),
                *bracket,
                *max_price,
                *limit,
                *owned,
                exclude,
                *json,
            )
        }
        Some(DeckCommand::Legal {
            name,
            format,
            bracket,
            json,
        }) => deck::legal::legal(paths, conn, out, name, format.as_deref(), *bracket, *json),
        Some(DeckCommand::Simulate {
            name,
            runs,
            turns,
            seed,
            format,
            baseline,
            hypgeo,
            combo,
            combo_limit,
            bracket,
            json,
        }) => deck::simulator::simulate(
            paths,
            conn,
            out,
            name,
            runs.unwrap_or(deck::simulator::DEFAULT_RUNS),
            *turns,
            *seed,
            format.as_deref(),
            baseline.as_deref(),
            *hypgeo,
            combo.clone(),
            combo_limit.map(|n| n as usize),
            *bracket,
            *json,
        ),
        Some(DeckCommand::Import {
            name,
            file,
            url,
            format,
        }) => deck::import(deck::io::ImportSource {
            paths,
            conn,
            out,
            json,
            name,
            file: file.as_deref(),
            url: url.as_deref(),
            format: format.as_deref(),
        }),
        Some(DeckCommand::Cuts {
            name,
            count,
            for_role,
            bracket,
            format,
            max_price,
            make_room_for,
            json,
        }) => deck::cuts(
            paths,
            conn,
            out,
            name,
            &deck::cuts::CutOptions {
                count: *count as usize,
                for_role: for_role.as_deref(),
                bracket: *bracket,
                format: format.as_deref(),
                max_price: *max_price,
                make_room_for: *make_room_for,
                json: *json,
            },
        ),
        Some(DeckCommand::Combos {
            name,
            format,
            bracket,
            json,
        }) => deck::combos(paths, conn, out, name, format.as_deref(), *bracket, *json),
        Some(DeckCommand::Diff {
            deck_a,
            deck_b,
            exact,
            json,
            markdown,
            as_update,
        }) => {
            if *as_update {
                return deck::diff_as_update(paths, conn, out, deck_a, deck_b, *exact);
            }
            let format = if *json {
                deck::diff::DiffFormat::Json
            } else if *markdown {
                deck::diff::DiffFormat::Markdown
            } else {
                deck::diff::DiffFormat::Human
            };
            deck::diff(paths, conn, out, deck_a, deck_b, *exact, format)
        }
        Some(DeckCommand::Export {
            name,
            file,
            force,
            format,
        }) => deck::export(paths, out, name, file, *force, format),
        Some(DeckCommand::Delete { name }) => deck::delete(paths, conn, out, name),
        Some(DeckCommand::Buylist { name, store, json }) => {
            deck::buylist(paths, conn, out, name, store.as_deref(), *json)
        }
        Some(DeckCommand::Copy {
            source,
            destination,
            force,
        }) => deck::copy(paths, out, source, destination, *force),
        Some(DeckCommand::Primer { name, set }) => deck::primer(paths, out, name, set.as_deref()),
    };
    // Deck-not-found is the one runtime error with a dedicated hint; emit
    // error+hint here and swallow it into an exit code.
    outcome.or_else(|err| {
        if deck::store::is_deck_not_found(&err) {
            out.error(&format!("{err:#}"));
            out.hint(
                "create it first: stm deck create <name>, or import: stm deck import <name> <file>",
            );
            Ok(codes::ERROR)
        } else {
            Err(err)
        }
    })
}

/// Stale-while-revalidate: when card data or prices are older than 24h,
/// refresh them before the command runs. The refresh is synchronous: a
/// stale store makes a read command block for the length of a full sync
/// (download + ingest + embed). `--offline` skips the check entirely;
/// a failure downgrades to a warning and never blocks the read.
fn revalidate_if_stale(paths: &paths::Paths, conn: &mut Connection, out: &mut Output) {
    if seizethemana::offline_requested() {
        return;
    }
    let status = match paths::Status::read(&paths.status_file()) {
        Ok(status) => status,
        Err(err) => {
            // An unreadable status file means the freshness gate has no
            // data; silence would hide a corrupt store. Never set up is
            // normal (setup prints its own error later); other failures
            // are worth a line on stderr.
            if paths.is_setup() {
                eprintln!(
                    "warning: status file unreadable ({err:#}); skipping the freshness check"
                );
            }
            return;
        }
    };
    let now = chrono::Utc::now();
    if !sync::is_stale(&status, now) {
        return;
    }
    out.status("Syncing", "card data and prices are stale, refreshing");
    let options = sync::SyncOptions { force: false };
    match sync::run_sync(paths, conn, out, &options) {
        Ok(_) => out.finish("Synced", "card data and prices", std::time::Duration::ZERO),
        // A failed background refresh must not block the read; the next
        // command retries.
        Err(err) => {
            out.warning(&format!(
                "background sync failed, showing possibly stale data: {err:#}"
            ));
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn offline_flag_is_process_state() {
        seizethemana::set_offline(false);
        assert!(!seizethemana::offline_requested());
        seizethemana::set_offline(true);
        assert!(seizethemana::offline_requested());
        seizethemana::set_offline(false);
    }
}
