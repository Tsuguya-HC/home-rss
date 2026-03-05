export interface Feed {
  id: string
  url: string
  title: string | null
  site_url: string | null
  etag: string | null
  last_modified: string | null
  last_fetched_at: number | null
  created_at: number | null
}

export interface Article {
  id: string
  feed_id: string
  url: string
  title: string
  content: string | null
  author: string | null
  published_at: number | null
  fetched_at: number | null
}

export interface Stats {
  feeds: number
  unread: number
}
