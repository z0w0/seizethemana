-- Set metadata (type, block, franchise) and print-level Universes Beyond
-- flags for the universe/franchise census.

ALTER TABLE sets ADD COLUMN set_type TEXT NOT NULL DEFAULT '';
ALTER TABLE sets ADD COLUMN block TEXT;
ALTER TABLE sets ADD COLUMN franchise TEXT;

ALTER TABLE card_prints ADD COLUMN universes_beyond INTEGER NOT NULL DEFAULT 0;