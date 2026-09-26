import { Feed, Article, Stats } from './types'

// server/src/lib.rs の UI_ADD_FEED_TIMEOUT_SECS / UI_TIMEOUT_MARGIN_SECS と
// 対になっている (#106 R8/U9)。サーバ側は
// `fetch_timeout_is_positive_and_matches_ui_expectation` テストで
// `FETCH_TIMEOUT.as_secs() * 2 + UI_TIMEOUT_MARGIN_SECS <=
// UI_ADD_FEED_TIMEOUT_SECS` を検査している。どちらかを変えたら両方見直すこと。
const UI_ADD_FEED_TIMEOUT_SECS = 45
const ADD_FEED_TIMEOUT_MS = UI_ADD_FEED_TIMEOUT_SECS * 1_000

async function request<T>(url: string, init?: RequestInit, timeoutMs = 30_000): Promise<T> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  try {
    const res = await fetch(url, { ...init, signal: controller.signal })
    if (!res.ok) {
      let message = `${res.status} ${res.statusText}`
      try {
        const body = await res.json()
        if (body.error) message = body.error
      } catch {
        // ignore parse error
      }
      throw new Error(message)
    }
    if (res.status === 204) return undefined as T
    return await res.json()
  } catch (e) {
    if (e instanceof DOMException && e.name === 'AbortError') {
      throw new Error('request timed out')
    }
    throw e
  } finally {
    clearTimeout(timer)
  }
}

export const api = {
  getFeeds: () => request<Feed[]>('/api/feeds'),

  addFeed: (url: string) =>
    request<Feed>(
      '/api/feeds',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ url }),
      },
      ADD_FEED_TIMEOUT_MS,
    ),

  deleteFeed: (id: string) =>
    request<void>(`/api/feeds/${id}`, { method: 'DELETE' }),

  getArticles: (feedId?: string | null, unread?: boolean, favoriteOnly?: boolean) => {
    const params = new URLSearchParams()
    if (feedId) params.set('feed_id', feedId)
    if (unread) params.set('unread', 'true')
    if (favoriteOnly) params.set('favorite', 'true')
    const query = params.toString()
    return request<Article[]>(`/api/articles${query ? `?${query}` : ''}`)
  },

  markRead: (id: string) =>
    request<void>(`/api/articles/${id}/read`, { method: 'POST' }),

  markFavorite: (id: string) =>
    request<void>(`/api/articles/${id}/favorite`, { method: 'POST' }),

  unmarkFavorite: (id: string) =>
    request<void>(`/api/articles/${id}/favorite`, { method: 'DELETE' }),

  markAllRead: () =>
    request<void>('/api/articles/read-all', { method: 'POST' }),

  importOpml: (file: File) =>
    // imported + already_present + skipped_invalid + skipped_blocked ==
    // OPML 内の xmlUrl 属性の総数 (#106 U8)。already_present: 既存の feed と
    // 同じ URL（ON CONFLICT DO NOTHING で 0 行）。skipped_invalid: xmlUrl が
    // 空/不正だった件数。skipped_blocked: SSRF ガードで拒否された件数 (#106
    // U2)。
    request<{
      imported: number
      already_present: number
      skipped_invalid: number
      skipped_blocked: number
    }>('/api/import/opml', {
      method: 'POST',
      headers: { 'Content-Type': 'application/xml' },
      body: file,
    }),

  getStats: () => request<Stats>('/api/stats'),
}
