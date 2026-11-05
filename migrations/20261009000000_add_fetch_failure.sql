-- migrate:up
ALTER TABLE feeds ADD COLUMN last_fetch_error TEXT;
ALTER TABLE feeds ADD COLUMN fetch_failing_since TIMESTAMPTZ;

-- migrate:down
ALTER TABLE feeds DROP COLUMN fetch_failing_since;
ALTER TABLE feeds DROP COLUMN last_fetch_error;
