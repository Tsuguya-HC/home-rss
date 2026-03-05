import { Feed, Article, Stats } from './types'

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init)
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
  return res.json()
}

export const api = {
  getFeeds: () => request<Feed[]>('/api/feeds'),

  addFeed: (url: string) =>
    request<Feed>('/api/feeds', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url }),
    }),

  deleteFeed: (id: string) =>
    request<void>(`/api/feeds/${id}`, { method: 'DELETE' }),

  getArticles: (feedId?: string | null, unread?: boolean) => {
    const params = new URLSearchParams()
    if (feedId) params.set('feed_id', feedId)
    if (unread) params.set('unread', 'true')
    const query = params.toString()
    return request<Article[]>(`/api/articles${query ? `?${query}` : ''}`)
  },

  markRead: (id: string) =>
    request<void>(`/api/articles/${id}/read`, { method: 'POST' }),

  markAllRead: () =>
    request<void>('/api/articles/read-all', { method: 'POST' }),

  importOpml: (file: File) =>
    request<{ imported: number }>('/api/import/opml', {
      method: 'POST',
      body: file,
    }),

  getStats: () => request<Stats>('/api/stats'),
}
