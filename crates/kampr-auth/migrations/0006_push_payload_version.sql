-- The payload version a subscription's client can read.
--
-- 2 for every row that already exists, because that is what they were written by: a client older
-- than payload v3 has one notification slot and posts whatever arrives into it, whatever tag the
-- payload carries. Sending it a `done` would overwrite a live question, so it is never sent one.
ALTER TABLE push_subscriptions ADD COLUMN payload_version INTEGER NOT NULL DEFAULT 2;
