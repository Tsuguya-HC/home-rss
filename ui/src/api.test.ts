import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from './api'

function stubFetch() {
  const fetchMock = vi.fn<(input: string, init?: RequestInit) => Promise<Response>>()
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

function okResponse<T>(data: T): Response {
  return { ok: true, status: 200, statusText: 'OK', json: async () => data } as Response
}

function errorResponse(status: number, statusText: string, json: () => Promise<unknown>): Response {
  return { ok: false, status, statusText, json } as Response
}

function hangingFetch(): (input: string, init?: RequestInit) => Promise<Response> {
  return (_input, init) =>
    new Promise<Response>((_resolve, reject) => {
      init?.signal?.addEventListener('abort', () => {
        reject(new DOMException('The operation was aborted.', 'AbortError'))
      })
    })
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('api', () => {
  it('fetches the feed list with a GET', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse([]))

    await expect(api.getFeeds()).resolves.toEqual([])

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock).toHaveBeenCalledWith('/api/feeds', {
      signal: expect.any(AbortSignal),
    })
  })

  it('adds a feed with a POST carrying the url as JSON', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse({ id: 'f1' }))

    await expect(api.addFeed('https://example.com/feed.xml')).resolves.toEqual({ id: 'f1' })

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock).toHaveBeenCalledWith('/api/feeds', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: 'https://example.com/feed.xml' }),
      signal: expect.any(AbortSignal),
    })
  })

  // A 204 has no body to parse, so this fails if the 204 branch is removed.
  it('deletes a feed with a DELETE and resolves a 204 without parsing a body', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue({
      ok: true,
      status: 204,
      statusText: 'No Content',
      json: async (): Promise<never> => {
        throw new Error('must not parse a 204 body')
      },
    } as unknown as Response)

    await expect(api.deleteFeed('f1')).resolves.toBeUndefined()

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock).toHaveBeenCalledWith('/api/feeds/f1', {
      method: 'DELETE',
      signal: expect.any(AbortSignal),
    })
  })

  it('fetches articles without a query string when no filter is given', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse([]))

    await api.getArticles()

    expect(fetchMock).toHaveBeenCalledWith('/api/articles', {
      signal: expect.any(AbortSignal),
    })
  })

  it('filters articles by feed', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse([]))

    await api.getArticles('f1')

    expect(fetchMock).toHaveBeenCalledWith('/api/articles?feed_id=f1', {
      signal: expect.any(AbortSignal),
    })
  })

  it('filters articles by feed and unread', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse([]))

    await api.getArticles('f1', true)

    expect(fetchMock).toHaveBeenCalledWith('/api/articles?feed_id=f1&unread=true', {
      signal: expect.any(AbortSignal),
    })
  })

  it('filters articles by unread without a feed', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse([]))

    await api.getArticles(null, true)

    expect(fetchMock).toHaveBeenCalledWith('/api/articles?unread=true', {
      signal: expect.any(AbortSignal),
    })
  })

  it('marks an article read with a POST', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse(null))

    await api.markRead('a1')

    expect(fetchMock).toHaveBeenCalledWith('/api/articles/a1/read', {
      method: 'POST',
      signal: expect.any(AbortSignal),
    })
  })

  it('marks all articles read with a POST', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse(null))

    await api.markAllRead()

    expect(fetchMock).toHaveBeenCalledWith('/api/articles/read-all', {
      method: 'POST',
      signal: expect.any(AbortSignal),
    })
  })

  it('imports OPML with a POST carrying the file as XML', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse({ imported: 1 }))
    const file = new File(['<opml version="1.0"></opml>'], 'feeds.opml', {
      type: 'application/xml',
    })

    await api.importOpml(file)

    expect(fetchMock).toHaveBeenCalledWith(
      '/api/import/opml',
      expect.objectContaining({
        method: 'POST',
        headers: { 'Content-Type': 'application/xml' },
        signal: expect.any(AbortSignal),
      }),
    )
    expect(fetchMock.mock.calls[0][1]?.body).toBe(file)
  })

  it('fetches stats with a GET', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(okResponse({}))

    await api.getStats()

    expect(fetchMock).toHaveBeenCalledWith('/api/stats', {
      signal: expect.any(AbortSignal),
    })
  })

  // Non-2xx bodies carry the message in `error`, so this fails if that field is ignored.
  it('throws the error field of a non-2xx JSON response', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(
      errorResponse(400, 'Bad Request', async () => ({ error: 'feed unreachable' })),
    )

    await expect(api.getFeeds()).rejects.toThrow('feed unreachable')
  })

  // A valid JSON body without an `error` field keeps the status line,
  // so this fails if the `if (body.error)` guard is dropped.
  it('throws the status line of a non-2xx JSON response without an error field', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(
      errorResponse(404, 'Not Found', async () => ({ message: 'no such feed' })),
    )

    await expect(api.getFeeds()).rejects.toThrow('404 Not Found')
  })

  // Without a JSON error the status line is the message, so this fails if the fallback changes.
  it('throws the status line of a non-2xx response without a JSON error', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockResolvedValue(
      errorResponse(500, 'Internal Server Error', async () => {
        throw new SyntaxError('not JSON')
      }),
    )

    await expect(api.getFeeds()).rejects.toThrow('500 Internal Server Error')
  })

  // Aborts surface as AbortError, so this fails if a different name is mapped.
  it('turns an abort into a timeout error', async () => {
    const fetchMock = stubFetch()
    fetchMock.mockRejectedValue(new DOMException('The operation was aborted.', 'AbortError'))

    await expect(api.getFeeds()).rejects.toThrow('request timed out')
  })

  // The default timeout is 30 seconds, so this fails if the duration changes.
  it('times out requests after 30 seconds', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('fetch', hangingFetch())
    let settled = false
    const pending = api.getFeeds()
    void pending.then(
      () => {
        settled = true
      },
      () => {
        settled = true
      },
    )
    await vi.advanceTimersByTimeAsync(29_999)
    expect(settled).toBe(false)
    await vi.advanceTimersByTimeAsync(1)
    await expect(pending).rejects.toThrow('request timed out')
    expect(settled).toBe(true)
  })

  // Adding a feed waits 45 seconds for the server-side fetch, so this fails
  // if addFeed falls back to the default timeout.
  it('gives adding a feed 45 seconds before timing out', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('fetch', hangingFetch())
    let settled = false
    const pending = api.addFeed('https://example.com/feed.xml')
    void pending.then(
      () => {
        settled = true
      },
      () => {
        settled = true
      },
    )
    await vi.advanceTimersByTimeAsync(30_000)
    expect(settled).toBe(false)
    await vi.advanceTimersByTimeAsync(15_000)
    await expect(pending).rejects.toThrow('request timed out')
  })
})
