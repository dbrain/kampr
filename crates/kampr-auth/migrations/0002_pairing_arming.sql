ALTER TABLE pairings ADD COLUMN armed_until INTEGER;

UPDATE pairings SET armed_until = expires_at;
