-- migrate:up
ALTER TABLE articles ADD COLUMN image_url TEXT;

-- migrate:down
ALTER TABLE articles DROP COLUMN image_url;
