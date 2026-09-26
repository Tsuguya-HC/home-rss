-- migrate:up
CREATE TABLE favorites (
  article_id UUID REFERENCES articles(id) ON DELETE CASCADE,
  favorited_at TIMESTAMPTZ DEFAULT now(),
  PRIMARY KEY (article_id)
);

-- migrate:down
DROP TABLE IF EXISTS favorites;
