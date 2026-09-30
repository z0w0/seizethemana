-- Sell-decision signals: Reserved List membership (never reprinted) and the
-- Penny Dreadful popularity rank. Both are nullable; NULL means the bulk did
-- not report the value, matching `edhrec_rank` and `game_changer`.

ALTER TABLE cards ADD COLUMN reserved   INTEGER;
ALTER TABLE cards ADD COLUMN penny_rank INTEGER;
