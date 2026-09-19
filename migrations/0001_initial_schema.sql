CREATE TABLE IF NOT EXISTS cards (
    id               INTEGER PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE,
    oracle_id        TEXT NOT NULL,
    mana_cost        TEXT NOT NULL DEFAULT '',
    cmc              REAL NOT NULL DEFAULT 0,
    type_line        TEXT NOT NULL DEFAULT '',
    colors           TEXT NOT NULL DEFAULT '[]',
    color_identity   TEXT NOT NULL DEFAULT '[]',
    keywords         TEXT NOT NULL DEFAULT '[]',
    power            TEXT,
    toughness        TEXT,
    loyalty          TEXT,
    oracle_text      TEXT NOT NULL DEFAULT '',
    rarity           TEXT NOT NULL DEFAULT '',
    edhrec_rank      INTEGER,
    legalities       TEXT NOT NULL DEFAULT '{}',
    set_code         TEXT NOT NULL DEFAULT '',
    collector_number TEXT NOT NULL DEFAULT '',
    scryfall_id      TEXT NOT NULL DEFAULT '',
    released_at      TEXT NOT NULL DEFAULT '',
    game_changer     INTEGER,
    tags_text        TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_cards_type     ON cards(type_line);

CREATE INDEX IF NOT EXISTS idx_cards_colors   ON cards(colors);

CREATE INDEX IF NOT EXISTS idx_cards_rarity   ON cards(rarity);

CREATE INDEX IF NOT EXISTS idx_cards_oracle   ON cards(oracle_id);



CREATE TABLE IF NOT EXISTS token_names (
    name TEXT PRIMARY KEY
);


-- Set code → full set name (Card Kingdom buylists match by name).
CREATE TABLE IF NOT EXISTS sets (
    set_code TEXT PRIMARY KEY,
    set_name TEXT NOT NULL
);


-- One row per physical printing: price and print info per Scryfall
-- print ID. Set codes are lowercase everywhere in this store.
-- Flavor-name prints ("Godzilla, King of the Monsters") carry the
-- just-for-fun name; name resolution treats it as an alias.
CREATE TABLE IF NOT EXISTS card_prints (
    scryfall_id      TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    flavor_name      TEXT NOT NULL DEFAULT '',
    set_code         TEXT NOT NULL,
    collector_number TEXT NOT NULL DEFAULT '',
    lang             TEXT NOT NULL DEFAULT 'en',
    rarity           TEXT NOT NULL DEFAULT '',
    finishes         TEXT NOT NULL DEFAULT '[]',
    released_at      TEXT NOT NULL DEFAULT '',
    usd              REAL,
    usd_foil         REAL,
    usd_etched       REAL,
    updated_at       TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_card_prints_name
    ON card_prints(name, set_code, collector_number);

-- Partial: alias lookups never touch the empty-string majority.
CREATE INDEX IF NOT EXISTS idx_card_prints_flavor
    ON card_prints(flavor_name) WHERE flavor_name != '';


-- Oracle tags from Scryfall's Tagger bulk (functional roles like
-- "removal" or "ramp"). `id` is the stable UUID; slugs and labels
-- can change between daily bulks, so only `id` is treated as a key.
-- `use_count` is the global tagging count, a popularity signal.
CREATE TABLE IF NOT EXISTS tags (
    id        TEXT PRIMARY KEY,
    slug      TEXT NOT NULL,
    label     TEXT NOT NULL,
    use_count INTEGER NOT NULL DEFAULT 0
);


-- Card-to-tag associations, keyed by oracle_id (cards.oracle_id).
-- Rewritten wholesale on every tags ingest.
CREATE TABLE IF NOT EXISTS card_tags (
    oracle_id TEXT NOT NULL,
    tag_id    TEXT NOT NULL,
    weight    TEXT NOT NULL DEFAULT 'median',
    PRIMARY KEY (oracle_id, tag_id)
);

CREATE INDEX IF NOT EXISTS idx_card_tags_oracle ON card_tags(oracle_id);

-- Reverse lookup: which cards carry one tag (`tag_hits_by_oracle`).
CREATE INDEX IF NOT EXISTS idx_card_tags_tag ON card_tags(tag_id);



CREATE TABLE IF NOT EXISTS collection (
    id               INTEGER PRIMARY KEY,
    name             TEXT NOT NULL,
    set_code         TEXT NOT NULL DEFAULT '',
    collector_number TEXT NOT NULL DEFAULT '',
    foil             TEXT NOT NULL DEFAULT 'normal'
        CHECK (foil IN ('normal', 'foil', 'etched')),
    binder           TEXT NOT NULL,
    binder_type      TEXT NOT NULL CHECK (binder_type IN ('binder', 'deck')),
    quantity         INTEGER NOT NULL DEFAULT 1,
    purchase_price   REAL NOT NULL DEFAULT 0,
    UNIQUE (name, set_code, collector_number, foil, binder, binder_type)
);

CREATE INDEX IF NOT EXISTS idx_collection_name   ON collection(name);

CREATE INDEX IF NOT EXISTS idx_collection_binder ON collection(binder);


-- Commander Spellbook combo variants, refreshed wholesale from the
-- bulk document on every sync. `legalities` is a JSON map of format
-- name → legal, so deck joins filter by the deck's format key.
CREATE TABLE IF NOT EXISTS combos (
    id                TEXT PRIMARY KEY,
    produces          TEXT NOT NULL DEFAULT '[]',
    mana_value_needed INTEGER NOT NULL DEFAULT 0,
    bracket_tag       TEXT,
    legalities        TEXT NOT NULL DEFAULT '{}',
    popularity        INTEGER,
    updated_at        TEXT NOT NULL
);


-- One row per combo piece. Faces ("A // B") store each half as its
-- own row so deck names match either face.
CREATE TABLE IF NOT EXISTS combo_pieces (
    combo_id          TEXT NOT NULL,
    name              TEXT NOT NULL,
    ordinal           INTEGER NOT NULL,
    zones             TEXT NOT NULL DEFAULT '[]',
    must_be_commander INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (combo_id, name, ordinal)
);

-- Candidate lookup for `combos::load_variants_for`: which combos contain
-- one of N card names.
CREATE INDEX IF NOT EXISTS idx_combo_pieces_name ON combo_pieces(name);



CREATE VIRTUAL TABLE IF NOT EXISTS cards_fts USING fts5(
    name, tags_text, type_line, oracle_text,
    content='cards', content_rowid='id',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS cards_fts_ai AFTER INSERT ON cards BEGIN
    INSERT INTO cards_fts(rowid, name, tags_text, type_line, oracle_text)
    VALUES (new.id, new.name, new.tags_text, new.type_line, new.oracle_text);

END;

CREATE TRIGGER IF NOT EXISTS cards_fts_ad AFTER DELETE ON cards BEGIN
    INSERT INTO cards_fts(cards_fts, rowid, name, tags_text, type_line, oracle_text)
    VALUES ('delete', old.id, old.name, old.tags_text, old.type_line, old.oracle_text);

END;

CREATE TRIGGER IF NOT EXISTS cards_fts_au AFTER UPDATE ON cards BEGIN
    INSERT INTO cards_fts(cards_fts, rowid, name, tags_text, type_line, oracle_text)
    VALUES ('delete', old.id, old.name, old.tags_text, old.type_line, old.oracle_text);

    INSERT INTO cards_fts(rowid, name, tags_text, type_line, oracle_text)
    VALUES (new.id, new.name, new.tags_text, new.type_line, new.oracle_text);

END;

