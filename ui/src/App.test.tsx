import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'

// Fixtures are typed as the JSON the server sends, not as the UI's types: the
// mocked fetch hands them over as a response body.
type Json = Record<string, unknown>

function feed(id: string, title: string): Json {
  return {
    id,
    url: `https://${id}.example/feed.xml`,
    title,
    site_url: null,
    etag: null,
    last_modified: null,
    last_fetched_at: null,
    created_at: null,
  }
}

function article(id: string, feedId: string, title: string): Json {
  return {
    id,
    feed_id: feedId,
    url: `https://${feedId}.example/${id}`,
    title,
    content: `<p>${title} body</p>`,
    author: null,
    published_at: null,
    fetched_at: null,
    image_url: null,
  }
}

const feedA = feed('fa', 'Feed A')
const feedB = feed('fb', 'Feed B')
const a1 = article('a1', 'fa', 'A1')
const a2 = article('a2', 'fa', 'A2')
const b1 = article('b1', 'fb', 'B1')

const UNREAD = '/api/articles?unread=true'

interface Pending {
  method: string
  url: string
  body: unknown
  resolve: (res: Response) => void
}

let pending: Pending[]
let issued: string[]

beforeEach(() => {
  pending = []
  issued = []
  vi.stubGlobal(
    'fetch',
    vi.fn(
      (url: string, init?: RequestInit) =>
        new Promise<Response>((resolve) => {
          const method = init?.method ?? 'GET'
          pending.push({ method, url, body: init?.body, resolve })
          issued.push(`${method} ${url}`)
        }),
    ),
  )
})

afterEach(() => {
  vi.unstubAllGlobals()
})

function response(status: number, data?: unknown): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: '',
    json: async () => {
      if (data === undefined) throw new SyntaxError('no body')
      return data
    },
  } as Response
}

async function take(method: string, url: string): Promise<Pending> {
  let found: Pending | undefined
  await waitFor(() => {
    const i = pending.findIndex((p) => p.method === method && p.url === url)
    if (i < 0) {
      throw new Error(
        `no pending ${method} ${url}; pending: ${pending.map((p) => `${p.method} ${p.url}`).join(', ')}`,
      )
    }
    found = pending.splice(i, 1)[0]
  })
  return found!
}

async function reply(req: Pending, status: number, data?: unknown) {
  await act(async () => {
    req.resolve(response(status, data))
  })
}

async function answer(method: string, url: string, status: number, data?: unknown) {
  await reply(await take(method, url), status, data)
}

function isPending(method: string, url: string) {
  return pending.some((p) => p.method === method && p.url === url)
}

// The unread counts and the unread list are the same query, so the server
// answers both alike.
async function mount(feeds: Json[], unread: Json[]) {
  render(<App />)
  await answer('GET', '/api/feeds', 200, feeds)
  await answer('GET', UNREAD, 200, unread)
  await answer('GET', UNREAD, 200, unread)
}

function feedRow(title: string): HTMLElement {
  return screen.getByText(title).closest('.feed-item') as HTMLElement
}

function badgeOf(row: HTMLElement): string | null {
  return row.querySelector('.unread-badge')?.textContent ?? null
}

function listedTitles(): string[] {
  return Array.from(document.querySelectorAll('.article-item-title')).map((e) => e.textContent!)
}

function detailTitle(): string | null {
  return document.querySelector('.detail-title')?.textContent ?? null
}

function errorBanner(): HTMLElement | null {
  return document.querySelector('.error-banner')
}

async function deleteFeed(user: ReturnType<typeof userEvent.setup>, title: string) {
  const button = within(feedRow(title)).getByRole('button', { name: '×' })
  await user.click(button)
  await user.click(within(feedRow(title)).getByRole('button', { name: '!' }))
}

async function submitFeed(user: ReturnType<typeof userEvent.setup>, url: string) {
  await user.click(screen.getByRole('button', { name: '+ フィード追加' }))
  await user.type(screen.getByPlaceholderText('https://example.com/feed.xml'), url)
  await user.click(screen.getByRole('button', { name: '追加' }))
}

describe('App', () => {
  describe('on start', () => {
    it('lists the feeds with their unread counts and the unread articles', async () => {
      render(<App />)
      expect(screen.getByText('読み込み中...')).toBeTruthy()

      await answer('GET', '/api/feeds', 200, [feedA, feedB])
      await answer('GET', UNREAD, 200, [a1, a2, b1])
      await answer('GET', UNREAD, 200, [a1, a2, b1])

      expect(issued).toEqual(['GET /api/feeds', `GET ${UNREAD}`, `GET ${UNREAD}`])
      expect(badgeOf(feedRow('Feed A'))).toBe('2')
      expect(badgeOf(feedRow('Feed B'))).toBe('1')
      expect(badgeOf(feedRow('すべて'))).toBe('3')
      expect(feedRow('すべて').className).toContain('feed-item--active')
      expect(listedTitles()).toEqual(['A1', 'A2', 'B1'])
      expect(screen.queryByText('読み込み中...')).toBeNull()
      expect(screen.getByText('記事を選択してください')).toBeTruthy()
      expect((screen.getByRole('checkbox', { name: '未読のみ' }) as HTMLInputElement).checked).toBe(
        true,
      )
    })

    // Feeds and counts are requested together; neither waits for the other.
    it.each([
      ['the feeds', 'the counts'],
      ['the counts', 'the feeds'],
    ])('shows the same sidebar when %s answer before %s', async (first) => {
      render(<App />)
      const feedsReq = await take('GET', '/api/feeds')
      const countsReq = await take('GET', UNREAD)
      const answers: [Pending, Json[]][] = [
        [feedsReq, [feedA, feedB]],
        [countsReq, [a1, b1]],
      ]
      if (first === 'the counts') answers.reverse()
      for (const [req, data] of answers) await reply(req, 200, data)

      expect(badgeOf(feedRow('Feed A'))).toBe('1')
      expect(badgeOf(feedRow('Feed B'))).toBe('1')
      expect(badgeOf(feedRow('すべて'))).toBe('2')
    })

    it('shows the error from the API and still loads the article list', async () => {
      render(<App />)
      await answer('GET', '/api/feeds', 500, { error: 'internal server error' })
      await answer('GET', UNREAD, 200, [a1])
      await answer('GET', UNREAD, 200, [a1])

      expect(errorBanner()?.textContent).toContain('internal server error')
      expect(screen.queryByText('Feed A')).toBeNull()
      expect(listedTitles()).toEqual(['A1'])
    })
  })

  describe('the error banner', () => {
    it('goes away when clicked', async () => {
      const user = userEvent.setup()
      render(<App />)
      await answer('GET', '/api/feeds', 500, { error: 'internal server error' })
      await answer('GET', UNREAD, 200, [])
      await answer('GET', UNREAD, 200, [])

      await user.click(errorBanner()!)

      expect(errorBanner()).toBeNull()
    })

    it('is cleared when the next action starts', async () => {
      const user = userEvent.setup()
      render(<App />)
      await answer('GET', '/api/feeds', 200, [feedA])
      await answer('GET', UNREAD, 200, [a1])
      await answer('GET', UNREAD, 500, { error: 'internal server error' })
      expect(errorBanner()).not.toBeNull()

      await user.click(screen.getByRole('button', { name: '全既読' }))

      expect(errorBanner()).toBeNull()
    })
  })

  describe('selecting a feed', () => {
    it("lists that feed's unread articles", async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, a2, b1])

      await user.click(screen.getByText('Feed B'))
      expect(screen.getByText('読み込み中...')).toBeTruthy()
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])

      expect(listedTitles()).toEqual(['B1'])
      expect(feedRow('Feed B').className).toContain('feed-item--active')
      expect(feedRow('すべて').className).not.toContain('feed-item--active')

      await user.click(screen.getByText('すべて'))
      await answer('GET', UNREAD, 200, [a1, a2, b1])

      expect(listedTitles()).toEqual(['A1', 'A2', 'B1'])
      expect(feedRow('すべて').className).toContain('feed-item--active')
    })

    it('closes the open article', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])
      await user.click(screen.getByText('A1'))
      await answer('POST', '/api/articles/a1/read', 204)

      await user.click(screen.getByText('Feed B'))

      expect(detailTitle()).toBeNull()
      expect(screen.getByText('記事を選択してください')).toBeTruthy()
    })

    it('shows the feed selected last when the answers come back in order', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await user.click(screen.getByText('Feed A'))
      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])

      expect(listedTitles()).toEqual(['B1'])
      expect(feedRow('Feed B').className).toContain('feed-item--active')
      expect(screen.queryByText('読み込み中...')).toBeNull()
    })

    // #276: the earlier answer used to overwrite the later one.
    it('shows the feed selected last when the earlier answer comes back last', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await user.click(screen.getByText('Feed A'))
      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])

      expect(listedTitles()).toEqual(['B1'])
      expect(feedRow('Feed B').className).toContain('feed-item--active')
      expect(screen.queryByText('読み込み中...')).toBeNull()
    })

    // #276: the earlier answer used to show its articles and end the loading.
    it('keeps loading until the feed selected last answers', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await user.click(screen.getByText('Feed A'))
      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])

      expect(screen.getByText('読み込み中...')).toBeTruthy()
      expect(listedTitles()).toEqual([])

      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])

      expect(listedTitles()).toEqual(['B1'])
      expect(screen.queryByText('読み込み中...')).toBeNull()
    })

    it('does not report a failure of a list it no longer shows', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await user.click(screen.getByText('Feed A'))
      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 500, {
        error: 'internal server error',
      })

      expect(errorBanner()).toBeNull()
      expect(screen.getByText('読み込み中...')).toBeTruthy()

      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])

      expect(listedTitles()).toEqual(['B1'])
    })
  })

  describe('opening an article', () => {
    it('shows it at once and marks it read when the server confirms', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, a2, b1])

      await user.click(screen.getByText('A1'))

      expect(detailTitle()).toBe('A1')
      expect(isPending('POST', '/api/articles/a1/read')).toBe(true)
      expect(badgeOf(feedRow('Feed A'))).toBe('2')
      expect(listedTitles()).toEqual(['A1', 'A2', 'B1'])

      await answer('POST', '/api/articles/a1/read', 204)

      expect(badgeOf(feedRow('Feed A'))).toBe('1')
      expect(badgeOf(feedRow('Feed B'))).toBe('1')
      expect(badgeOf(feedRow('すべて'))).toBe('2')
      // #107: the unread-only list keeps what was just read until it is reloaded.
      expect(listedTitles()).toEqual(['A1', 'A2', 'B1'])
      expect(detailTitle()).toBe('A1')
    })

    it('keeps every article read in the unread-only list until the list is reloaded', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, a2, b1])

      await user.click(screen.getByText('A1'))
      await answer('POST', '/api/articles/a1/read', 204)
      await user.click(screen.getByText('A2'))
      await answer('POST', '/api/articles/a2/read', 204)

      expect(listedTitles()).toEqual(['A1', 'A2', 'B1'])
      expect(badgeOf(feedRow('Feed A'))).toBeNull()
      expect(badgeOf(feedRow('すべて'))).toBe('1')

      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])
      await user.click(screen.getByText('すべて'))
      await answer('GET', UNREAD, 200, [b1])

      expect(listedTitles()).toEqual(['B1'])
    })

    it('keeps it unread and shows the error when marking it read fails', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1, a2])

      await user.click(screen.getByText('A1'))
      await answer('POST', '/api/articles/a1/read', 500, { error: 'internal server error' })

      expect(errorBanner()?.textContent).toContain('internal server error')
      expect(badgeOf(feedRow('Feed A'))).toBe('2')
      expect(listedTitles()).toEqual(['A1', 'A2'])
      expect(detailTitle()).toBe('A1')
    })

    // The read is confirmed for a feed that is no longer on screen.
    it.each([['before'], ['after']])(
      "counts the read against the article's feed when it is confirmed %s the next feed's list arrives",
      async (when) => {
        const user = userEvent.setup()
        await mount([feedA, feedB], [a1, a2, b1])

        await user.click(screen.getByText('A1'))
        const read = await take('POST', '/api/articles/a1/read')
        await user.click(screen.getByText('Feed B'))
        const list = await take('GET', '/api/articles?feed_id=fb&unread=true')
        const answers: [Pending, number, Json[]?][] = [
          [read, 204],
          [list, 200, [b1]],
        ]
        if (when === 'after') answers.reverse()
        for (const [req, status, data] of answers) await reply(req, status, data)

        expect(badgeOf(feedRow('Feed A'))).toBe('1')
        expect(badgeOf(feedRow('Feed B'))).toBe('1')
        expect(listedTitles()).toEqual(['B1'])
      },
    )
  })

  describe('the unread-only filter', () => {
    it('lists read articles too when turned off, and closes the open article', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1, a2])
      await user.click(screen.getByText('A1'))
      await answer('POST', '/api/articles/a1/read', 204)
      expect(listedTitles()).toEqual(['A1', 'A2'])

      await user.click(screen.getByRole('checkbox', { name: '未読のみ' }))

      expect(detailTitle()).toBeNull()
      await answer('GET', '/api/articles', 200, [a1, a2])
      expect(listedTitles()).toEqual(['A1', 'A2'])
      expect((screen.getByRole('checkbox', { name: '未読のみ' }) as HTMLInputElement).checked).toBe(
        false,
      )

      // With the filter off, reading an article keeps it in the list.
      await user.click(screen.getByText('A2'))
      await answer('POST', '/api/articles/a2/read', 204)
      expect(listedTitles()).toEqual(['A1', 'A2'])

      await user.click(screen.getByRole('checkbox', { name: '未読のみ' }))
      await answer('GET', UNREAD, 200, [])
      expect(screen.getByText('記事がありません')).toBeTruthy()
    })

    it('applies to the selected feed', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])
      await user.click(screen.getByText('Feed A'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])

      await user.click(screen.getByRole('checkbox', { name: '未読のみ' }))
      await answer('GET', '/api/articles?feed_id=fa', 200, [a1, a2])

      expect(listedTitles()).toEqual(['A1', 'A2'])
    })
  })

  describe('marking everything read', () => {
    it('clears the counts and the unread list once the server confirms', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await user.click(screen.getByRole('button', { name: '全既読' }))
      expect(listedTitles()).toEqual(['A1', 'B1'])
      await answer('POST', '/api/articles/read-all', 204)

      expect(badgeOf(feedRow('Feed A'))).toBeNull()
      expect(badgeOf(feedRow('Feed B'))).toBeNull()
      expect(badgeOf(feedRow('すべて'))).toBeNull()
      await answer('GET', UNREAD, 200, [])
      expect(screen.getByText('記事がありません')).toBeTruthy()
    })

    // #107: marking everything read is a reload, so it also drops articles
    // that were opened and kept in the list.
    it('drops the articles opened before from the unread-only list', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1, a2])
      await user.click(screen.getByText('A1'))
      await answer('POST', '/api/articles/a1/read', 204)

      await user.click(screen.getByRole('button', { name: '全既読' }))
      await answer('POST', '/api/articles/read-all', 204)
      await answer('GET', UNREAD, 200, [])

      expect(screen.getByText('記事がありません')).toBeTruthy()
    })

    it('reloads the list for the selection made while the server was confirming', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await user.click(screen.getByRole('button', { name: '全既読' }))
      const readAll = await take('POST', '/api/articles/read-all')
      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])
      await reply(readAll, 204)
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [])

      expect(issued.filter((r) => r === `GET ${UNREAD}`)).toHaveLength(2)
      expect(screen.getByText('記事がありません')).toBeTruthy()
    })

    it('changes nothing when the server refuses', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1])

      await user.click(screen.getByRole('button', { name: '全既読' }))
      await answer('POST', '/api/articles/read-all', 500, { error: 'internal server error' })

      expect(errorBanner()?.textContent).toContain('internal server error')
      expect(badgeOf(feedRow('Feed A'))).toBe('1')
      expect(listedTitles()).toEqual(['A1'])
    })
  })

  describe('adding a feed', () => {
    const NEW_URL = 'https://fb.example/feed.xml'

    it('posts the url, then reloads the feeds and the list', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1])

      await submitFeed(user, NEW_URL)
      const post = await take('POST', '/api/feeds')
      expect(JSON.parse(post.body as string)).toEqual({ url: NEW_URL })
      expect(screen.getByRole('button', { name: '追加中...' })).toBeTruthy()
      // Nothing is reloaded until the add itself answers.
      expect(isPending('GET', '/api/feeds')).toBe(false)

      await reply(post, 201, feedB)
      await answer('GET', '/api/feeds', 200, [feedB, feedA])
      await answer('GET', UNREAD, 200, [a1, b1])
      await answer('GET', UNREAD, 200, [a1, b1])

      expect(badgeOf(feedRow('Feed B'))).toBe('1')
      expect(listedTitles()).toEqual(['A1', 'B1'])
      expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()
      expect(errorBanner()).toBeNull()
    })

    // #276: the reload used the selection from when the add was sent.
    it('reloads the list for the feed selected while the add was in flight', async () => {
      const user = userEvent.setup()
      const feedC = feed('fc', 'Feed C')
      await mount([feedA, feedB], [a1, b1])
      await user.click(screen.getByText('Feed A'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])

      await submitFeed(user, 'https://fc.example/feed.xml')
      const post = await take('POST', '/api/feeds')
      await user.click(screen.getByRole('button', { name: 'キャンセル' }))
      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])

      await reply(post, 201, feedC)
      await answer('GET', '/api/feeds', 200, [feedA, feedB, feedC])
      await answer('GET', UNREAD, 200, [a1, b1])
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])

      expect(isPending('GET', '/api/articles?feed_id=fa&unread=true')).toBe(false)
      expect(feedRow('Feed B').className).toContain('feed-item--active')
      expect(listedTitles()).toEqual(['B1'])
    })

    it('still reloads the lists, and shows why, when the add fails', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1])

      await submitFeed(user, NEW_URL)
      await answer('POST', '/api/feeds', 502, { error: 'failed to fetch feed' })
      await answer('GET', '/api/feeds', 200, [feedA])
      await answer('GET', UNREAD, 200, [a1])
      await answer('GET', UNREAD, 200, [a1])

      expect(errorBanner()?.textContent).toContain('failed to fetch feed')
      expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()
      expect(listedTitles()).toEqual(['A1'])
    })

    it('reports a failed list reload alongside a failed add', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1])

      await submitFeed(user, NEW_URL)
      await answer('POST', '/api/feeds', 502, { error: 'failed to fetch feed' })
      await answer('GET', '/api/feeds', 200, [feedA])
      await answer('GET', UNREAD, 200, [a1])
      await answer('GET', UNREAD, 500, { error: 'internal server error' })

      expect(errorBanner()?.textContent).toContain(
        'failed to fetch feed（記事一覧の再読み込みにも失敗しました）',
      )
    })

    it('says both lists failed to reload after a successful add', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1])

      await submitFeed(user, NEW_URL)
      await answer('POST', '/api/feeds', 201, feedB)
      await answer('GET', '/api/feeds', 500, { error: 'feeds unavailable' })
      await answer('GET', UNREAD, 200, [a1])
      await answer('GET', UNREAD, 500, { error: 'articles unavailable' })

      expect(errorBanner()?.textContent).toContain(
        'フィード一覧・記事一覧の再読み込みに失敗しました: feeds unavailable',
      )
    })

    it('reports only the feed list when only it fails to reload', async () => {
      const user = userEvent.setup()
      await mount([feedA], [a1])

      await submitFeed(user, NEW_URL)
      await answer('POST', '/api/feeds', 201, feedB)
      await answer('GET', '/api/feeds', 500, { error: 'feeds unavailable' })
      await answer('GET', UNREAD, 200, [a1, b1])
      await answer('GET', UNREAD, 200, [a1, b1])

      expect(errorBanner()?.textContent).toContain('feeds unavailable')
      expect(errorBanner()?.textContent).not.toContain('再読み込み')
      expect(listedTitles()).toEqual(['A1', 'B1'])
    })
  })

  describe('deleting a feed', () => {
    it('deletes it after the confirming click and reloads the feeds and the list', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await user.click(within(feedRow('Feed B')).getByRole('button', { name: '×' }))
      expect(isPending('DELETE', '/api/feeds/fb')).toBe(false)
      await user.click(within(feedRow('Feed B')).getByRole('button', { name: '!' }))
      await answer('DELETE', '/api/feeds/fb', 204)
      await answer('GET', '/api/feeds', 200, [feedA])
      await answer('GET', UNREAD, 200, [a1])
      await answer('GET', UNREAD, 200, [a1])

      expect(screen.queryByText('Feed B')).toBeNull()
      expect(badgeOf(feedRow('すべて'))).toBe('1')
      expect(listedTitles()).toEqual(['A1'])
    })

    it('goes back to all feeds when the selected feed is deleted', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])
      await user.click(screen.getByText('Feed A'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])
      await user.click(screen.getByText('A1'))
      await answer('POST', '/api/articles/a1/read', 204)

      await deleteFeed(user, 'Feed A')
      await answer('DELETE', '/api/feeds/fa', 204)

      expect(feedRow('すべて').className).toContain('feed-item--active')
      expect(detailTitle()).toBeNull()
      // The unread counts, and the list for all feeds.
      expect(pending.filter((p) => p.method === 'GET' && p.url === UNREAD)).toHaveLength(2)
    })

    // #276: the deleted feed's list was requested last and emptied this one.
    it('lists all feeds once the selected feed is deleted', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])
      await user.click(screen.getByText('Feed A'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])

      await deleteFeed(user, 'Feed A')
      await answer('DELETE', '/api/feeds/fa', 204)
      await answer('GET', '/api/feeds', 200, [feedB])
      await answer('GET', UNREAD, 200, [b1])
      await answer('GET', UNREAD, 200, [b1])

      expect(isPending('GET', '/api/articles?feed_id=fa&unread=true')).toBe(false)
      expect(listedTitles()).toEqual(['B1'])
      expect(screen.queryByText('読み込み中...')).toBeNull()
    })

    it('keeps the feed selected while the delete was in flight', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])
      await user.click(screen.getByText('Feed A'))
      await answer('GET', '/api/articles?feed_id=fa&unread=true', 200, [a1])

      await deleteFeed(user, 'Feed A')
      const del = await take('DELETE', '/api/feeds/fa')
      await user.click(screen.getByText('Feed B'))
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])
      await reply(del, 204)
      await answer('GET', '/api/feeds', 200, [feedB])
      await answer('GET', UNREAD, 200, [b1])
      await answer('GET', '/api/articles?feed_id=fb&unread=true', 200, [b1])

      expect(feedRow('Feed B').className).toContain('feed-item--active')
      expect(listedTitles()).toEqual(['B1'])
    })

    it('keeps the feed and shows the error when the server refuses', async () => {
      const user = userEvent.setup()
      await mount([feedA, feedB], [a1, b1])

      await deleteFeed(user, 'Feed B')
      await answer('DELETE', '/api/feeds/fb', 404, { error: 'feed not found' })

      expect(errorBanner()?.textContent).toContain('feed not found')
      expect(screen.getByText('Feed B')).toBeTruthy()
      expect(isPending('GET', '/api/feeds')).toBe(false)
    })
  })
})
